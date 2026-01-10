use std::collections::{HashMap, HashSet};

use bigdecimal::BigDecimal;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    U256,
    crypto::{PublicKey, Signature},
    error::{BtcError, Result},
    sha256::Hash,
    util::MerkelRoot,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlockChain {
    utxos: HashMap<Hash, (bool, TransactionOutput)>,
    target: U256,
    blocks: Vec<Block>,
    #[serde(default, skip_serializing)]
    mempool: Vec<Transaction>,
}

impl BlockChain {
    pub fn new() -> Self {
        BlockChain {
            blocks: Vec::new(),
            utxos: HashMap::new(),
            target: crate::MIN_TARGET,
            mempool: vec![],
        }
    }

    pub fn rebuild_utxos(&mut self) {
        for block in &self.blocks {
            for tx in &block.transactions {
                for input in &tx.inputs {
                    self.utxos.remove(&input.prev_tx_output_hash);
                }

                for output in tx.outputs.iter() {
                    self.utxos.insert(tx.hash(), (false, output.clone()));
                }
            }
        }
    }

    pub fn add_block(&mut self, block: Block) -> Result<()> {
        if self.blocks.is_empty() {
            if block.header.prev_block_hash != Hash::zero() {
                println!("zero hash");
                return Err(BtcError::InvalidBlock);
            }
        } else {
            let last_block = self.blocks.last().unwrap();

            if block.header.prev_block_hash != last_block.hash() {
                println!("prev hash is wrong");
                return Err(BtcError::InvalidBlock);
            }

            if !block.header.hash().matches_target(block.header.target) {
                println!("target does not match");
                return Err(BtcError::InvalidBlock);
            }

            let calculated_merkel_root = MerkelRoot::calculate(&block.transactions);

            if calculated_merkel_root != block.header.merkle_root {
                println!("merkel root does not match");
                return Err(BtcError::InvalidMerkleRoot);
            }

            if block.header.timestamp <= last_block.header.timestamp {
                return Err(BtcError::InvalidBlock);
            }

            block.verify_transactions(self.block_height(), self.utxos())?;
        }

        let block_transactions: HashSet<_> =
            block.transactions.iter().map(|tx| tx.hash()).collect();

        self.mempool
            .retain(|tx| !block_transactions.contains(&tx.hash()));

        self.blocks.push(block);
        self.try_adjust_target();
        Ok(())
    }

    pub fn try_adjust_target(&mut self) {
        if self.blocks.is_empty() {
            return;
        }

        if !self
            .blocks
            .len()
            .is_multiple_of(crate::DIFICULTY_UPDATE_INTERVAL as usize)
        {
            return;
        }

        let start_time = self.blocks[self.blocks.len() - crate::DIFICULTY_UPDATE_INTERVAL as usize]
            .header
            .timestamp;

        let end_time = self.blocks.last().unwrap().header.timestamp;

        let time_diff = end_time - start_time;

        let time_diff_in_seconds = time_diff.num_seconds();

        let target_seconds = crate::IDEAL_BLOCK_TIME * crate::DIFICULTY_UPDATE_INTERVAL;

        let new_target = BigDecimal::parse_bytes(self.target.to_string().as_bytes(), 10)
            .expect("BUG: impossible")
            * (BigDecimal::from(time_diff_in_seconds) / BigDecimal::from(target_seconds));

        let new_target_str = new_target
            .to_string()
            .split('.')
            .next()
            .expect("BUG: expected decimal type")
            .to_owned();

        let new_target: U256 = U256::from_str_radix(&new_target_str, 10).expect("BUG: impossible");

        let new_target = if new_target < self.target / 4 {
            self.target / 4
        } else if new_target > self.target * 4 {
            self.target * 4
        } else {
            new_target
        };

        self.target = new_target.min(crate::MIN_TARGET);
    }

    pub fn add_to_mempool(&mut self, tx: Transaction) -> Result<()> {
        let mut known_inputs = HashSet::new();
        for input in &tx.inputs {
            if !self.utxos.contains_key(&input.prev_tx_output_hash) {
                return Err(BtcError::InvalidTransaction);
            }

            if known_inputs.contains(&input.prev_tx_output_hash) {
                return Err(BtcError::InvalidTransaction);
            }

            known_inputs.insert(input.prev_tx_output_hash);
        }

        for input in &tx.inputs {
            if let Some((true, _)) = self.utxos.get(&input.prev_tx_output_hash) {
                let referencing_tx = self.mempool.iter().enumerate().find(|(_, transaction)| {
                    transaction
                        .outputs
                        .iter()
                        .any(|output| output.hash() == input.prev_tx_output_hash)
                });

                if let Some((idx, referencing_tx)) = referencing_tx {
                    for input in &referencing_tx.inputs {
                        self.utxos
                            .entry(input.prev_tx_output_hash)
                            .and_modify(|(marked, _)| {
                                *marked = false;
                            });
                    }

                    self.mempool.remove(idx);
                } else {
                    self.utxos.entry(input.prev_tx_output_hash).and_modify(|(marked, _)| {
                        *marked = false;
                    });
                }
            }
        }

        let all_inputs = tx
            .inputs
            .iter()
            .map(|input| {
                self.utxos
                    .get(&input.prev_tx_output_hash)
                    .expect("BUG: impossible")
                    .1
                    .value
            })
            .sum::<u64>();

        let all_outputs = tx.outputs.iter().map(|output| output.value).sum::<u64>();

        if all_inputs < all_outputs {
            return Err(BtcError::InvalidTransaction);
        }

        self.mempool.push(tx);

        self.mempool.sort_by_key(|transaction| {
            let all_input: u64 = transaction
                .inputs
                .iter()
                .map(|input| {
                    self.utxos
                        .get(&input.prev_tx_output_hash)
                        .expect("BUG: impossible")
                        .1
                        .value
                })
                .sum();

            let all_output: u64 = transaction.outputs.iter().map(|output| output.value).sum();

            all_input - all_output // sort by miner fees
        });
        Ok(())
    }

    pub fn utxos(&self) -> &HashMap<Hash, (bool, TransactionOutput)> {
        &self.utxos
    }

    pub fn target(&self) -> U256 {
        self.target
    }

    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.blocks.iter()
    }

    pub fn block_height(&self) -> u64 {
        self.blocks.len() as u64
    }

    pub fn mempool(&self) -> &[Transaction] {
        &self.mempool
    }
}

impl Default for BlockChain {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
}

impl Block {
    pub fn new(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        Self {
            header,
            transactions,
        }
    }

    pub fn hash(&self) -> Hash {
        Hash::hash(self)
    }

    pub fn verify_transactions(
        &self,
        predicted_block_height: u64,
        utxos: &HashMap<Hash, (bool, TransactionOutput)>,
    ) -> Result<()> {
        let mut inputs = HashMap::new();

        if self.transactions.is_empty() {
            return Err(BtcError::InvalidTransaction);
        }

        self.verify_coinbase_transaction(predicted_block_height, utxos)?;

        for tx in self.transactions.iter().skip(1) {
            let mut input_value = 0;
            let mut output_value = 0;

            for input in &tx.inputs {
                let prev_output = utxos
                    .get(&input.prev_tx_output_hash)
                    .map(|(_, output)| output);

                if prev_output.is_none() {
                    return Err(BtcError::InvalidTransaction);
                }

                let prev_output = prev_output.unwrap();

                if inputs.contains_key(&input.prev_tx_output_hash) {
                    return Err(BtcError::InvalidTransaction);
                }

                if !input
                    .signature
                    .verify(&input.prev_tx_output_hash, &prev_output.pubkey)
                {
                    return Err(BtcError::InvalidSignature);
                }

                input_value += prev_output.value;
                inputs.insert(input.prev_tx_output_hash, prev_output.clone());
            }

            for output in &tx.outputs {
                output_value += output.value;
            }

            if input_value < output_value {
                return Err(BtcError::InvalidTransaction);
            }
        }

        Ok(())
    }

    pub fn verify_coinbase_transaction(
        &self,
        predicted_block_height: u64,
        utxos: &HashMap<Hash, (bool, TransactionOutput)>,
    ) -> Result<()> {
        let coinbase_transaction = &self.transactions[0];

        if !coinbase_transaction.inputs.is_empty() {
            return Err(BtcError::InvalidTransaction);
        }

        if coinbase_transaction.outputs.is_empty() {
            return Err(BtcError::InvalidTransaction);
        }

        let miner_fees = self.calculate_miner_fees(utxos)?;
        let block_reward = crate::INITIAL_REWARD * 10u64.pow(8)
            / 2u64.pow((predicted_block_height / crate::HALVING_INTERVAL) as u32);

        let total_coinbase_outputs: u64 = coinbase_transaction
            .outputs
            .iter()
            .map(|output| output.value)
            .sum();

        if total_coinbase_outputs != block_reward + miner_fees {
            return Err(BtcError::InvalidTransaction);
        }
        Ok(())
    }

    pub fn calculate_miner_fees(
        &self,
        utxos: &HashMap<Hash, (bool, TransactionOutput)>,
    ) -> Result<u64> {
        let mut inputs = HashMap::new();
        let mut outputs = HashMap::new();

        for transaction in self.transactions.iter().skip(1) {
            for input in &transaction.inputs {
                let prev_output = utxos
                    .get(&input.prev_tx_output_hash)
                    .map(|(_, output)| output);

                if prev_output.is_none() {
                    return Err(BtcError::InvalidTransaction);
                }

                let prev_output = prev_output.unwrap();

                if inputs.contains_key(&input.prev_tx_output_hash) {
                    return Err(BtcError::InvalidTransaction);
                }

                inputs.insert(input.prev_tx_output_hash, prev_output.clone());
            }

            for output in &transaction.outputs {
                if outputs.contains_key(&output.hash()) {
                    return Err(BtcError::InvalidTransaction);
                }
                outputs.insert(output.hash(), output.clone());
            }
        }

        let inputs_value: u64 = inputs.values().map(|output| output.value).sum();
        let outputs_value: u64 = outputs.values().map(|output| output.value).sum();

        Ok(inputs_value - outputs_value)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlockHeader {
    pub timestamp: DateTime<Utc>,
    pub nonce: u64,
    pub prev_block_hash: Hash,
    pub merkle_root: MerkelRoot,
    pub target: U256,
}

impl BlockHeader {
    pub fn new(
        timestamp: DateTime<Utc>,
        nonce: u64,
        prev_block_hash: Hash,
        merkle_root: MerkelRoot,
        target: U256,
    ) -> Self {
        Self {
            timestamp,
            nonce,
            prev_block_hash,
            merkle_root,
            target,
        }
    }

    pub fn hash(&self) -> Hash {
        Hash::hash(self)
    }

    pub fn mine(&mut self, steps: usize) -> bool {
        if self.hash().matches_target(self.target) {
            return true;
        }

        for _ in 0..steps {
            if let Some(new_nonce) = self.nonce.checked_add(1) {
                self.nonce = new_nonce;
            } else {
                self.nonce = 0;
                self.timestamp = Utc::now();
            }

            if self.hash().matches_target(self.target) {
                return true;
            }
        }
        false
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Transaction {
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
}

impl Transaction {
    pub fn new(inputs: Vec<TransactionInput>, outputs: Vec<TransactionOutput>) -> Self {
        Self { inputs, outputs }
    }

    pub fn hash(&self) -> Hash {
        Hash::hash(self)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TransactionInput {
    pub prev_tx_output_hash: Hash,
    pub signature: Signature,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TransactionOutput {
    pub value: u64,
    pub unique_id: Uuid,
    pub pubkey: PublicKey,
}

impl TransactionOutput {
    pub fn hash(&self) -> Hash {
        Hash::hash(self)
    }
}
