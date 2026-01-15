use btclib::network::Message;
use btclib::sha256::Hash;
use btclib::types::{Block, BlockHeader, Transaction, TransactionOutput};
use btclib::util::MerkleRoot;
use chrono::Utc;
use tokio::net::TcpStream;
use uuid::Uuid;

pub async fn handle_connection(mut socket: TcpStream) {
    loop {
        let message = match Message::recv_async(&mut socket).await {
            Ok(message) => message,
            Err(e) => {
                println!("invalid message from peer: {}, closing the connection", e);
                return;
            }
        };

        use btclib::network::Message::*;

        match message {
            UTXOs(_) | Template(_) | Difference(_) | TemplateValidity(_) | NodeList(_) => {
                println!("These are for miners and wallets");
                return;
            }

            FetchBlock(height) => {
                let blockchain = crate::BLOCKCHAIN.read().await;
                let Some(block) = blockchain.blocks().nth(height as usize).cloned() else {
                    return;
                };

                let message = NewBlock(block);
                message.send_async(&mut socket).await.unwrap();
            }

            DiscoverNodes => {
                let nodes = crate::NODES
                    .iter()
                    .map(|x| x.key().clone())
                    .collect::<Vec<_>>();

                let message = NodeList(nodes);
                message.send_async(&mut socket).await.unwrap();
            }

            AskDifference(height) => {
                let blockchain = crate::BLOCKCHAIN.read().await;
                let count = blockchain.block_height() as i32 - height as i32;
                let message = Difference(count);
                message.send_async(&mut socket).await.unwrap();
            }

            FetchUTXOs(key) => {
                println!("received request to fetch UTXOs");

                let blockchain = crate::BLOCKCHAIN.read().await;
                let utxos = blockchain
                    .utxos()
                    .iter()
                    .filter(|(_, (_, txout))| txout.pubkey == key)
                    .map(|(_, (marked, txout))| (txout.clone(), *marked))
                    .collect::<Vec<_>>();

                let message = UTXOs(utxos);
                message.send_async(&mut socket).await.unwrap();
            }

            NewBlock(block) => {
                let mut blockchain = crate::BLOCKCHAIN.write().await;
                println!("received new block");

                if blockchain.add_block(block).is_err() {
                    println!("block rejected");
                }
            }

            NewTransaction(tx) => {
                let mut blockchain = crate::BLOCKCHAIN.write().await;
                println!("received new transaction");

                if blockchain.add_to_mempool(tx).is_err() {
                    println!("transaction rejected, closing connection");
                    return;
                }
            }

            ValidateTemplate(block_template) => {
                let blockchain = crate::BLOCKCHAIN.write().await;

                let status = block_template.header.prev_block_hash
                    == blockchain
                        .blocks()
                        .last()
                        .map(|last_block| last_block.hash())
                        .unwrap_or(Hash::zero());

                let message = TemplateValidity(status);
                message.send_async(&mut socket).await.unwrap();
            }

            SubmitTemplate(block) => {
                println!("Received mined template");

                let mut blockchain = crate::BLOCKCHAIN.write().await;

                if let Err(e) = blockchain.add_block(block.clone()) {
                    println!("block rejected: {}, closing connection", e);
                    return;
                }

                blockchain.rebuild_utxos();

                println!("Good block, broadcasting to peers");
                let nodes = crate::NODES
                    .iter()
                    .map(|x| x.key().clone())
                    .collect::<Vec<_>>();

                for node in nodes {
                    if let Some(mut stream) = crate::NODES.get_mut(&node) {
                        let message = NewBlock(block.clone());
                        if message.send_async(&mut *stream).await.is_err() {
                            println!("failed to broadcast to {}", node);
                        }
                    }
                }
            }

            SubmitTransaction(tx) => {
                println!("submit tx");

                let mut blockchain = crate::BLOCKCHAIN.write().await;

                if let Err(e) = blockchain.add_to_mempool(tx.clone()) {
                    println!("transaction rejected: {}, closing connection", e);
                    return;
                }

                println!("added transaction to mempool");

                let nodes = crate::NODES
                    .iter()
                    .map(|x| x.key().clone())
                    .collect::<Vec<_>>();

                for node in nodes {
                    println!("broadcasting to {}", node);

                    if let Some(mut stream) = crate::NODES.get_mut(&node) {
                        let message = NewTransaction(tx.clone());
                        if message.send_async(&mut *stream).await.is_err() {
                            println!("failed to broadcast to {}", node);
                        }
                    }

                    println!("broadcasted to {}", node);
                }
            }

            FetchTemplate(pubkey) => {
                let blockchain = crate::BLOCKCHAIN.read().await;

                let mut transactions = vec![];

                transactions.extend(
                    blockchain
                        .mempool()
                        .iter()
                        .take(btclib::BLOCK_TRANSACTION_CAP)
                        .map(|(_, tx)| tx)
                        .cloned()
                        .collect::<Vec<_>>(),
                );

                let miner_fees = blockchain.calculate_fees(&transactions);
                let reward = blockchain.calculate_block_reward();

                transactions.insert(
                    0,
                    Transaction {
                        inputs: vec![],
                        outputs: vec![TransactionOutput {
                            pubkey,
                            unique_id: Uuid::new_v4(),
                            value: reward + miner_fees,
                        }],
                    },
                );

                let merkle_root = MerkleRoot::calculate(&transactions);

                let block = Block::new(
                    BlockHeader {
                        timestamp: Utc::now(),
                        nonce: 0,
                        prev_block_hash: blockchain
                            .blocks()
                            .last()
                            .map(|last_block| last_block.hash())
                            .unwrap_or(Hash::zero()),
                        merkle_root,
                        target: blockchain.target(),
                    },
                    transactions,
                );

                let message = Template(block);
                message.send_async(&mut socket).await.unwrap();
            }
        }
    }
}
