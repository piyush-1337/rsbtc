use std::collections::{HashMap, HashSet};

use bigdecimal::BigDecimal;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use std::io::{Error as IoError, ErrorKind as IoErrorKind, Read, Result as IoResult, Write};

use crate::{
    U256,
    error::{BtcError, Result},
    sha256::Hash,
    types::{
        block::Block,
        transaction::{Transaction, TransactionOutput},
    },
    util::{MerkleRoot, Savable},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlockChain {
    utxos: HashMap<Hash, (bool, TransactionOutput)>,
    target: U256,
    blocks: Vec<Block>,
    #[serde(default, skip_serializing)]
    mempool: Vec<(DateTime<Utc>, Transaction)>,
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

            let calculated_merkel_root = MerkleRoot::calculate(&block.transactions);

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
            .retain(|(_, tx)| !block_transactions.contains(&tx.hash()));

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
                let referencing_tx =
                    self.mempool
                        .iter()
                        .enumerate()
                        .find(|(_, (_, transaction))| {
                            transaction
                                .outputs
                                .iter()
                                .any(|output| output.hash() == input.prev_tx_output_hash)
                        });

                if let Some((idx, (_, referencing_tx))) = referencing_tx {
                    for input in &referencing_tx.inputs {
                        self.utxos
                            .entry(input.prev_tx_output_hash)
                            .and_modify(|(marked, _)| {
                                *marked = false;
                            });
                    }

                    self.mempool.remove(idx);
                } else {
                    self.utxos
                        .entry(input.prev_tx_output_hash)
                        .and_modify(|(marked, _)| {
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
            print!("all inputs are less than all outputs");
            return Err(BtcError::InvalidTransaction);
        }

        for input in &tx.inputs {
            self.utxos
                .entry(input.prev_tx_output_hash)
                .and_modify(|(marked, _)| {
                    *marked = true;
                });
        }

        self.mempool.push((Utc::now(), tx));

        self.mempool.sort_by_key(|(_, transaction)| {
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

    pub fn cleanup_mempool(&mut self) {
        let now = Utc::now();
        let mut utxo_hashes_to_unmark = vec![];

        self.mempool.retain(|(timestamp, tx)| {
            if now - *timestamp
                > chrono::Duration::seconds(crate::MAX_MEMPOOL_TRANSACTION_AGE as i64)
            {
                utxo_hashes_to_unmark
                    .extend(tx.inputs.iter().map(|input| input.prev_tx_output_hash));
                false
            } else {
                true
            }
        });

        for hash in utxo_hashes_to_unmark {
            self.utxos.entry(hash).and_modify(|(marked, _)| {
                *marked = false;
            });
        }
    }

    pub fn calculate_block_reward(&self) -> u64 {
        let block_height = self.block_height();
        let halvings = block_height / crate::HALVING_INTERVAL;
        (crate::INITIAL_REWARD * 10u64.pow(8)) >> halvings
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

    pub fn mempool(&self) -> &[(DateTime<Utc>, Transaction)] {
        &self.mempool
    }
}

impl Default for BlockChain {
    fn default() -> Self {
        Self::new()
    }
}

impl Savable for BlockChain {
    fn load<I: Read>(reader: I) -> IoResult<Self> {
        ciborium::de::from_reader(reader)
            .map_err(|_| IoError::new(IoErrorKind::InvalidData, "Failed to deserialize BlockChain"))
    }

    fn save<O: Write>(&self, writer: O) -> IoResult<()> {
        ciborium::ser::into_writer(self, writer)
            .map_err(|_| IoError::new(IoErrorKind::InvalidData, "Failed to serialize BlockChain"))
    }
}
