# rsbtc

A simplified Bitcoin implementation written in Rust, featuring peer-to-peer networking, mining, and wallet functionality. This project serves as a functional demonstration of blockchain principles, including Proof of Work, Merkle roots, and consensus mechanisms.

## Project Structure

The project is organized as a Rust workspace with three primary components and a shared library:

- **node**: The core blockchain engine that handles peer-to-peer communication, validates blocks/transactions, and maintains the ledger.
- **miner**: A dedicated binary for Proof of Work computation that connects to a node.
- **wallet**: A client interface for managing keys and initiating transactions.
- **lib**: Shared logic for cryptography (SHA-256), networking, types, and utility binaries for manual generation/inspection.

---

## Features

- **P2P Networking**: Nodes communicate to reach consensus on the canonical chain.
- **Proof of Work**: Mining implementation using SHA-256.
- **Merkle Roots**: Used for efficient transaction verification within blocks.
- **Key Management**: Support for generating and using public/private keys.
- **CBOR Serialization**: Binary serialization for blocks and keys.
- **Logging**: Integrated logging for monitoring node and wallet activity.

---

## Prerequisites

- **Rust**: Version 1.92.0 was used for development, but it is compatible with version 1.80.0+.
- No external system dependencies are required.

---

## Getting Started

Note that you need to have the node running before the running the miner or wallet.

### 1. Setup Data and Keys

You can use the helper binaries in the `lib` directory to generate initial keys and blocks:

```bash
cd lib
# Available binaries: key_gen, block_gen, tx_gen, block_print, tx_print
cargo run --bin key_gen

```

### 2. Running the Node

The node listens on `0.0.0.0:9000` by default.

```bash
cd node
cargo run

```

Usage: node [<nodes...>] [--port <port>] [--blockchain-file <blockchain-file>]

Positional Arguments:
  nodes             address of initial nodes

Options:
  --port            port number
  --blockchain-file path to the blockchain
  --help, help      display usage information

### 3. Running the Miner

Connect the miner to a running node and specify a public key to receive mining rewards.

```bash
cd miner
cargo run -- -a <node_address> -p <your_public_key>

```
Usage: miner --address <ADDRESS> --public-key-file <PUBLIC_KEY_FILE>

Options:
  -a, --address <ADDRESS>
  -p, --public-key-file <PUBLIC_KEY_FILE>
  -h, --help                               Print help
  -V, --version                            Print version

### 4. Running the Wallet

The wallet uses `wallet_config.toml` for configuration. A template is provided in the `wallet/` directory.

```bash
cd wallet
cargo run

```
Usage: wallet [OPTIONS] [COMMAND]

Commands:
  generate-config
  help             Print this message or the help of the given subcommand(s)

Options:
  -c, --config <FILE>   [default: wallet_config.toml]
  -n, --node <ADDRESS>
  -h, --help            Print help
  -V, --version         Print version

---

## Technical Details

While this implementation covers core blockchain concepts, it is designed for educational purposes:

- **Consensus**: Uses the longest chain rule via P2P communication.
- **Data Integrity**: Implements SHA-256 hashing and Merkle trees.
- **Simplified Logic**: Does not include a scripting system (Bitcoin Script), SegWit, or SPV (Simplified Payment Verification) clients.
