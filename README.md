# stress4844

Tiny CLI for submitting large calldata transactions to EVM networks to stress test the networking layer. Main motivation: evaluating how the pre-Shanghai/Capella network handles I/O load in advance of [EIP4844](https://eips.ethereum.org/EIPS/eip-4844) blobs.

The CLI expects a signing key and RPC provider, as well as a bundle signer if going through mev-boost. These can be entered in the [setup script](setup_env.sh).

## Running the Script

We support 2 modes of transaction submission - transactions can be generated and submitted in parallel to the mempool, or bundled for inclusion via [MEV-boost](https://boost.flashbots.net/).

The MEV-boost route allows the user to bid for inclusion, potentially crowding out other transactions in high demand environments such as ETH Mainnet. However, it requires participating in a first price auction for transaction inclusion, which requires some subjective determination of how much to bid. It is also subject to the percentage of proposers that run mev-boost, which was [~50% on Goerli](https://boost-relay-goerli.flashbots.net) as of January 2023.

To submit large calldata transactions through mev-boost, run

```
cargo r -- --rpc-url $ETH_RPC_URL --tx-signer $SIGNER --bundle-signer $BUNDLE  --chunk-size 1734  --fill-pct 80 --tip-wei $(cast --to-unit 3gwei) --blocks 18
```

`ETH_RPC_URL`, `SIGNER`, and `BUNDLE` environment variables are set and applied in `setup_env.sh`.

`--fill-pct` is a value in [0, 100] which sets what percentage of the 2MB block limit our bundles will fill. We have not successfully landed any bundles that requested more than 89% of a block.

`--chunk-size` sets the size of the calldata _per transaction_, in KB. Our bundle signer was explicitly whitelisted by the flashbots relay in order to submit transactions exceeding the usual 128kb limit.

`--tip-wei` determines how much to overbid on the gas for the transactions - this determines the "bribe" amount received by the proposer. In practice we have found reasonable landing rates with ~5gwei on Goerli.

The example command uses [Foundry Cast](https://book.getfoundry.sh/cast/) to convert from gwei to wei; you may alternatively pass in a value of wei directly.

`--blocks` sets how many bundles to land. The script will keep sending bundles until this many have landed successfully.

To submit large calldata transactions through the mempool, run

```
cargo r -- --rpc-url $ETH_RPC_URL --tx-signer $SIGNER --chunk-size 128 --mempool-txs 128 --mem-pool
```

`--chunk-size` again sets the size of the calldata per transaction in KB. Geth enforces a maximum of 128kb for mempool propagation.

`--mempool-txs` sets how many transactions to pre-sign and submit. These will be submitted to the RPC provider simultaneously.

`--mem-pool` is a boolean flag that indicates we want to submit directly to the mempool.

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
