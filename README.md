# stress4844

Tiny CLI for submitting large calldata transactions to EVM networks to stress test the networking layer. Main motivation: [EIP4844](https://eips.ethereum.org/EIPS/eip-4844) blobs.

```
cargo build --release
./target/release/stress4844 --rpc-url $ETH_RPC_URL -p $PRIVKEY`
```

## CLI Help

Pick a private key and an RPC URL for the network you're stress testing, and ensure you have some ETH. We default to 100wei per transaction for testnets, so you shouldn't need much.


```bash
./target/release/stress4844 --help
Usage: stress4844 [OPTIONS] --rpc-url <RPC_URL> --tx-signer <TX_SIGNER> --bundle-signer <BUNDLE_SIGNER> --payment <PAYMENT>

Options:
      --blocks <BLOCKS>                The number of blocks to run the stress test for [default: 1]
  -f, --fill-pct <FILL_PCT>            What % of the block to fill (0-100) [default: 100]
  -c, --chunk-size <CHUNK_SIZE>        [default: 128]
  -r, --rpc-url <RPC_URL>              The HTTP RPC endpoint to submit the transactions to
  -t, --tx-signer <TX_SIGNER>          The private key for the wallet you'll submit the stress test transactions with. MUST have enough ETH to cover for the gas
  -b, --bundle-signer <BUNDLE_SIGNER>  The private key for the full-block template bundle signer wallet
  -g, --gas-price <GAS_PRICE>          [default: 100]
  -p, --payment <PAYMENT>
  -h, --help                           Print help information
```
