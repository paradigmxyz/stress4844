// Eth libs
use ethers::prelude::{signer::SignerMiddleware, *};
use ethers_flashbots::{BundleRequest, FlashbotsMiddleware};

// CLI
use clap::Parser;
use tracing_subscriber::{filter::EnvFilter, prelude::*};

// Misc
use rand::{distributions::Standard, Rng};
use std::sync::Arc;
use url::Url;

#[derive(Debug, Parser)]
struct Opts {
    #[arg(default_value = "1", long)]
    /// The number of blocks to run the stress test for
    blocks: usize,

    #[arg(default_value = "100", long, short, value_parser = clap::value_parser!(u8).range(1..=100))]
    /// What % of the block to fill (0-100).
    fill_pct: u8,

    #[arg(default_value = "128", long, short)]
    chunk_size: usize,

    /// The HTTP RPC endpoint to submit the transactions to.
    #[arg(long, short, value_parser = http_provider)]
    rpc_url: String,

    /// The private key for the wallet you'll submit the stress test
    /// transactions with. MUST have enough ETH to cover for the gas.
    #[arg(long, short)]
    tx_signer: String,

    /// The private key for the full-block template bundle signer wallet.
    #[arg(long, short)]
    bundle_signer: String,

    #[arg(default_value = "100", long, short)]
    gas_price: U256,
}

fn http_provider(s: &str) -> Result<String, String> {
    if s.starts_with("http://") || s.starts_with("https://") {
        Ok(s.to_string())
    } else {
        Err(format!("URL does not start with http(s): {s}",))
    }
}

// From: https://github.com/ethereum/go-ethereum/blob/c2e0abce2eedc1ba2a1b32c46fd07ef18a25354a/core/txpool/txpool.go#L44-L55
/// `TX_SLOT_SIZE` is used to calculate how many data slots a single transaction
/// takes up based on its size. The slots are used as DoS protection, ensuring
/// that validating a new transaction remains a constant operation (in reality
/// O(maxslots), where max slots are 4 currently).
const _TX_SLOT_SIZE: usize = 32 * 1024;

/// txMaxSize is the maximum size a single transaction can have. This field has
/// non-trivial consequences: larger transactions are significantly harder and
/// more expensive to propagate; larger transactions also take more resources
/// to validate whether they fit into the pool or not.
const _TX_MAX_SIZE: usize = 4 * _TX_SLOT_SIZE; // 128KB

/// 1 kilobyte = 1024 bytes
const KB: usize = 1024;

/// Arbitrarily chosen number to cover for nonce+from+to+gas price size in a serialized
/// transaction
const TRIM_BYTES: usize = 500;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let opts = Opts::parse();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::new("stress4844=trace"))
        .init();

    let provider = Arc::new(Provider::try_from(opts.rpc_url)?);
    let signer = opts
        .tx_signer
        .strip_prefix("0x")
        .unwrap_or(&opts.tx_signer)
        .parse::<LocalWallet>()?;

    let bundle_signer = opts
        .bundle_signer
        .strip_prefix("0x")
        .unwrap_or(&opts.bundle_signer)
        .parse::<LocalWallet>()?;

    let bundle_middleware = FlashbotsMiddleware::new(
        provider.clone(),
        Url::parse("https://boost-relay-goerli.flashbots.net/")?,
        bundle_signer,
    );

    let address = signer.address();
    let balance = provider.get_balance(address, None).await?;
    let block = provider
        .get_block(BlockNumber::Latest)
        .await?
        .expect("could not get latest block");
    let mut nonce = provider
        .get_transaction_count(address, Some(BlockNumber::Pending.into()))
        .await?;

    tracing::info!(
        "starting benchmark from {:?} (balance: {} ETH, nonce: {})",
        address,
        ethers::core::utils::format_units(balance, "gwei")?,
        nonce
    );
    tracing::debug!("block gas limit: {} gas", block.gas_limit);
    let provider = SignerMiddleware::new_with_provider_chain(bundle_middleware, signer).await?;
    let chain_id = provider.signer().chain_id();

    // TODO: Do we want this to be different per transaction?
    let receiver: Address = "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".parse()?;

    // `CHUNKS_SIZE` Kilobytes per transaction, shave off 500 bytes to leave room for
    // the other fields to be serialized.
    let chunk = opts.chunk_size * KB - TRIM_BYTES;

    // Sample junk data for the blob.
    let blob = rand::thread_rng()
        .sample_iter(Standard)
        // .take(chunks)
        .take(10) // TODO: Remove.
        .collect::<Vec<u8>>();
    let blob_len = blob.len();

    // Craft the transaction.
    let tx = TransactionRequest::new()
        .chain_id(chain_id)
        .value(0)
        .from(address)
        .to(receiver)
        .data(blob)
        .gas_price(opts.gas_price);

    let gas_per_tx = provider.estimate_gas(&tx.clone().into(), None).await?;
    tracing::debug!("tx cost {} gas", gas_per_tx);

    // For each block, we want `fill_pct` -> we generate N transactions to reach that.
    let gas_used_per_block = block.gas_limit * opts.fill_pct / 100;
    let txs_per_block = 2; //  (gas_used_per_block / gas_per_tx).as_u64();
    tracing::info!(
        "submitting {txs_per_block} {blob_len} KB txs per block for {} blocks",
        opts.blocks
    );

    // Construct the bundle
    let mut bundle = BundleRequest::new();
    for _ in 0..txs_per_block {
        let mut tx = tx.clone();

        // increment the nonce and apply it
        nonce += 1.into();
        tx.nonce = Some(nonce);
        tx.gas = Some(gas_per_tx);

        // make into typed tx for the signer
        let tx = tx.into();
        let signature = provider.signer().sign_transaction(&tx).await?;
        let rlp = tx.rlp_signed(&signature);
        bundle = bundle.push_transaction(rlp);
    }

    tracing::info!("signed {} transactions", txs_per_block);

    // configure the bundle to fire it away
    let block_number = provider.get_block_number().await?;
    bundle = bundle
        .set_block(block_number + 1)
        .set_simulation_block(block_number)
        .set_simulation_timestamp(0);

    let simulated_bundle = provider.inner().simulate_bundle(&bundle).await?;
    tracing::info!("simulated {:?}", simulated_bundle);

    tracing::debug!("Done! End Block: {}", provider.get_block_number().await?);

    Ok(())
}
