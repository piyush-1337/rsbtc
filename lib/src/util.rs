use serde::{Deserialize, Serialize};

use crate::sha256::Hash;
use crate::types::Transaction;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MerkelRoot(Hash);

impl MerkelRoot {
    pub fn calculate(transactions: &[Transaction]) -> Self {
        let mut layer = vec![];

        for transaction in transactions {
            layer.push(Hash::hash(transaction));
        }

        while layer.len() > 1 {
            let mut new_layer = vec![];
            for pair in layer.chunks(2) {
                let left = pair[0];
                let right = pair.get(1).unwrap_or(&pair[0]);
                new_layer.push(Hash::hash(&[left, *right]));
            }
            layer = new_layer;
        }

        Self(layer[0])

    }
}
