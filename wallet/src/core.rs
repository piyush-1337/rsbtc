use anyhow::Result;
use btclib::crypto::{PrivateKey, PublicKey};
use btclib::network::Message;
use btclib::types::{Transaction, TransactionOutput};
use btclib::util::Savable;
use crossbeam_skiplist::SkipMap;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::*;

#[derive(Serialize, Deserialize, Clone)]
pub struct Key {
    pub public: PathBuf,
    pub private: PathBuf,
}

#[derive(Clone)]
struct LoadedKey {
    public: PublicKey,
    private: PrivateKey,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Recipient {
    pub name: String,
    pub key: PathBuf,
}

#[derive(Clone)]
pub struct LoadedRecipient {
    pub name: String,
    pub key: PublicKey,
}

impl Recipient {
    pub fn load(&self) -> Result<LoadedRecipient> {
        let key = PublicKey::load_from_file(&self.key)?;
        Ok(LoadedRecipient {
            name: self.name.clone(),
            key,
        })
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub enum FeeType {
    Fixed,
    Percent,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FeeConfig {
    pub fee_type: FeeType,
    pub value: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub my_keys: Vec<Key>,
    pub contacts: Vec<Recipient>,
    pub default_node: String,
    pub fee_config: FeeConfig,
}

#[derive(Clone)]
struct UtxoStore {
    my_keys: Vec<LoadedKey>,
    utxos: Arc<SkipMap<PublicKey, Vec<(bool, TransactionOutput)>>>,
}

impl UtxoStore {
    fn new() -> Self {
        Self {
            my_keys: Vec::new(),
            utxos: Arc::new(SkipMap::new()),
        }
    }

    fn add_key(&mut self, key: LoadedKey) {
        self.my_keys.push(key);
    }
}

pub struct Core {
    pub config: Config,
    utxos: UtxoStore,
    pub tx_sender: kanal::Sender<Transaction>,
    pub stream: Mutex<TcpStream>,
}

impl Core {
    fn new(config: Config, utxos: UtxoStore, steam: TcpStream) -> Self {
        let (tx_sender, _) = kanal::bounded(10);

        Self {
            config,
            utxos,
            tx_sender,
            stream: Mutex::new(steam),
        }
    }

    pub async fn load(config_path: PathBuf) -> Result<Self> {
        info!("Loading config from config: {:?}", config_path);
        let config: Config = toml::from_str(&fs::read_to_string(&config_path)?)?;
        let mut utxos = UtxoStore::new();
        let steam = TcpStream::connect(&config.default_node).await?;

        for key in &config.my_keys {
            let public = PublicKey::load_from_file(&key.public)?;
            let private = PrivateKey::load_from_file(&key.private)?;

            utxos.add_key(LoadedKey { public, private });
        }

        Ok(Self::new(config, utxos, steam))
    }

    pub async fn fetch_utxos(&self) -> Result<()> {
        debug!("Fetching UTXOs from node: {}", self.config.default_node);

        for key in &self.utxos.my_keys {
            let message = Message::FetchUTXOs(key.public.clone());
            message.send_async(&mut *self.stream.lock().await).await?;

            if let Message::UTXOs(utxos) =
                Message::recv_async(&mut *self.stream.lock().await).await?
            {
                debug!("Received {} UTXOs for key: {:?}", utxos.len(), key.public);

                self.utxos.utxos.insert(
                    key.public.clone(),
                    utxos
                        .into_iter()
                        .map(|(output, marked)| (marked, output))
                        .collect(),
                );
            } else {
                error!("Unexpected response from node");
                return Err(anyhow::anyhow!("Unexpected response from node"));
            }
        }
        info!("UTXOs fetched successfully");
        Ok(())
    }

    pub async fn send_transaction(&self, transaction: Transaction) -> Result<()> {
        debug!("Sending transaction to node: {}", self.config.default_node);

        let message = Message::SubmitTransaction(transaction);
        message.send_async(&mut *self.stream.lock().await).await?;

        info!("Transaction sent successfully");
        Ok(())
    }

    pub fn send_transaction_async(&self, recipient: &str, amount: u64) -> Result<()> {
        info!("Preparing to send {} satoshis to {}", amount, recipient);

        let recipient_key = self
            .config
            .contacts
            .iter()
            .find(|r| r.name == recipient)
            .ok_or_else(|| anyhow::anyhow!("Recipient not found"))?
            .load()?
            .key;

        let transaction = self.create_transaction(&recipient_key, amount)?;
        debug!("Sending transaction asynchronously");

        self.tx_sender.send(transaction)?;

        Ok(())
    }

    pub fn get_balance(&self) -> u64 {
        self.utxos
            .utxos
            .iter()
            .map(|entry| entry.value().iter().map(|utxo| utxo.1.value).sum::<u64>())
            .sum()
    }

    pub fn create_transaction(&self, recipient: &PublicKey, amount: u64) -> Result<Transaction> {
        let fee = self.calculate_fee(amount);
        let total_amount = amount + fee;
        let mut inputs = Vec::new();
        let mut input_sum = 0;

        for entry in self.utxos.utxos.iter() {
            let pubkey = entry.key();
            let utxos = entry.value();

            for (marked, utxo) in utxos.iter() {
                if *marked {
                    continue;
                }

                if input_sum >= total_amount {
                    break;
                }

                inputs.push(btclib::types::TransactionInput {
                    prev_tx_output_hash: utxo.hash(),
                    signature: btclib::crypto::Signature::sign_output(
                        &utxo.hash(),
                        &self
                            .utxos
                            .my_keys
                            .iter()
                            .find(|k| k.public == *pubkey)
                            .unwrap()
                            .private,
                    ),
                });

                input_sum += utxo.value;
            }

            if input_sum >= total_amount {
                break;
            }
        }

        if input_sum < total_amount {
            return Err(anyhow::anyhow!("Insufficient funds"));
        }

        let mut outputs = vec![TransactionOutput {
            value: amount,
            unique_id: uuid::Uuid::new_v4(),
            pubkey: recipient.clone(),
        }];

        if input_sum > total_amount {
            outputs.push(TransactionOutput {
                value: input_sum - total_amount,
                unique_id: uuid::Uuid::new_v4(),
                pubkey: self.utxos.my_keys[0].public.clone(),
            });
        }

        Ok(Transaction::new(inputs, outputs))
    }

    fn calculate_fee(&self, amount: u64) -> u64 {
        match self.config.fee_config.fee_type {
            FeeType::Fixed => self.config.fee_config.value as u64,

            FeeType::Percent => (amount as f64 * self.config.fee_config.value / 100.0) as u64,
        }
    }
}
