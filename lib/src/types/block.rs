use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use std::io::{Error as IoError, ErrorKind as IoErrorKind, Read, Result as IoResult, Write};

use crate::{
    U256,
    error::{BtcError, Result},
    sha256::Hash,
    types::{Transaction, TransactionOutput},
    util::{MerkelRoot, Savable},
};

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

impl Savable for Block {
    fn load<I: Read>(reader: I) -> IoResult<Self> {
        ciborium::de::from_reader(reader)
            .map_err(|_| IoError::new(IoErrorKind::InvalidData, "Failed to deserialize Block"))
    }

    fn save<O: Write>(&self, writer: O) -> IoResult<()> {
        ciborium::ser::into_writer(self, writer)
            .map_err(|_| IoError::new(IoErrorKind::InvalidData, "Failed to serialize Block"))
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
