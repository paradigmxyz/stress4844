// CLI
use clap::Parser;
use eyre::Result;
use serde_json::{json, Value};
use tracing_subscriber::{filter::EnvFilter, prelude::*};

// Misc
use chrono::prelude::*;
use ethers::prelude::*;
use ethers_flashbots::FlashbotsMiddleware;
use std::fs::OpenOptions;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

// local utils
mod bundle_builder;

#[derive(Debug, Parser)]
struct Opts {
    #[arg(default_value = "1", long)]
    /// The number of blocks to run the stress test for
    blocks: usize,

    #[arg(default_value = "99", long, short, value_parser = clap::value_parser!(u8).range(1..=100))]
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

    #[arg(default_value = "6000000000", long)] // default "tip" is 6gwei.
    tip_wei: u64, // have noticed that on goerli, inclusion seems to be pretty
                  // insensitive to the bribe/tip amount
}

fn http_provider(s: &str) -> Result<String, String> {
    if s.starts_with("http://") || s.starts_with("https://") {
        Ok(s.to_string())
    } else {
        Err(format!("URL does not start with http(s): {s}",))
    }
}

fn get_attempt_json(chunk_size: usize, tip_wei: u64, fill_pct: u8, success: bool) -> Value {
    let entry = json!({
            "tip_wei": tip_wei,
            "fill_pct": fill_pct,
            "success": success,
            "time": Utc::now().to_string(),
            "chunk_size": chunk_size,
    });
    return entry;
}

fn log_attempt(chunk_size: usize, tip_wei: u64, fill_pct: u8, success: bool) {
    let _entry = get_attempt_json(chunk_size, tip_wei, fill_pct, success);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("stress-4844-attempts.json")
        .unwrap();

    let res = serde_json::to_writer(file, &_entry);

    match res {
        Err(e) => eprintln!("Couldn't write to file: {}", e),
        Ok(_) => return,
    }
}

/// Address of the following contract to allow for easy coinbase payments on Goerli.
///
/// contract CoinbasePayer {
///     receive() payable external {
///         payable(address(block.coinbase)).transfer(msg.value);
///     }
/// }
// const COINBASE_PAYER_ADDR: &str = "0x060d6635bb76c71871f97C12f10Fa20BD8e87eC0";

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let opts = Opts::parse();
    let interval = Duration::from_secs(1);
    let rpc_url = opts.rpc_url;
    let tx_signer = opts.tx_signer.strip_prefix("0x").unwrap_or(&opts.tx_signer);
    let bundle_signer = opts
        .bundle_signer
        .strip_prefix("0x")
        .unwrap_or(&opts.bundle_signer);
    let mut landed = 0;
    let blocks_to_land = opts.blocks;
    let fill_pct = opts.fill_pct; // how much of the full 2MB payload to take up with calldata
    let tip_wei = opts.tip_wei; // how much to overpay on gas, in percentage points

    let chunk_size = opts.chunk_size;
    //let chunk_size = ethers::prelude::U256::from(opts.chunk_size); // for example, 384, 512, etc.

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::new("stress4844=trace"))
        .init();

    let bundle_signer = bundle_signer.parse::<LocalWallet>()?;

    let provider: Arc<Provider<Http>> =
        Arc::new(Provider::<Http>::try_from(rpc_url)?.interval(interval));

    let signer = tx_signer.parse::<LocalWallet>()?;

    let bundle_middleware = FlashbotsMiddleware::new(
        provider.clone(),
        Url::parse("https://relay-goerli.flashbots.net/")?, // TODO: make configurable
        bundle_signer,
    );

    let address = signer.address();
    let balance = provider.get_balance(address, None).await?;

    tracing::info!(
        "starting benchmark from {:?} (balance: {} ETH)",
        address,
        ethers::core::utils::format_units(balance, "eth")?,
    );
    let provider =
        Arc::new(SignerMiddleware::new_with_provider_chain(bundle_middleware, signer).await?);
    let chain_id = provider.signer().chain_id();

    let mut nonce = provider
        .get_transaction_count(address, Some(BlockNumber::Pending.into()))
        .await?;
    tracing::debug!("current nonce: {}", nonce);
    // TODO: Do we want this to be different per transaction?
    let receiver: Address = "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".parse()?;

    let block = provider
        .get_block(BlockNumber::Latest)
        .await?
        .expect("could not get latest block");

    let mut bundle = bundle_builder::construct_bundle(
        chain_id,
        address,
        receiver,
        provider.clone(),
        block.gas_limit,
        fill_pct,
        nonce,
        chunk_size,
        tip_wei,
    )
    .await?;
    // should always be 30 million:
    // tracing::debug!("block gas limit: {} gas", block.gas_limit);

    // on every block try to get the bundle in
    let mut block_sub = provider.watch_blocks().await?;
    tracing::info!("subscribed to blocks - waiting for next");
    while block_sub.next().await.is_some() && landed <= blocks_to_land {
        let block_number = provider.get_block_number().await?;
        let block = provider
            .get_block(BlockNumber::Latest)
            .await?
            .expect("could not get latest block");
        //tracing::debug!("block gas limit: {} gas", block.gas_limit);

        let span = tracing::trace_span!("submit-bundle", block = block_number.as_u64());
        let _enter = span.enter();

        bundle = bundle
            .set_block(block_number + 1)
            .set_simulation_block(block_number)
            .set_simulation_timestamp(0);
        tracing::debug!("bundle target block {:?}", block_number + 1);

        let pending_bundle = provider.inner().send_bundle(&bundle).await?;
        match pending_bundle.await {
            Ok(bundle_hash) => {
                // TODO: Can we log more info from the Flashbots API?
                tracing::info!("bundle #{} included! hash: {:?}", landed, bundle_hash);

                landed += 1; // actually check if we landed it?
                log_attempt(chunk_size, tip_wei, fill_pct, true);
            }
            Err(err) => {
                tracing::error!("{}. did not land bundle, retrying.", err);
                log_attempt(chunk_size, tip_wei, fill_pct, false);
            }
        }
        nonce = provider
            .get_transaction_count(address, Some(BlockNumber::Pending.into()))
            .await?; // TODO: keep track of nonce ourselves?
        tracing::debug!("signing new bundle for next block (new nonce: {})", nonce);
        bundle = bundle_builder::construct_bundle(
            chain_id,
            address,
            receiver,
            provider.clone(),
            block.gas_limit,
            fill_pct,
            nonce,
            chunk_size,
            tip_wei,
        )
        .await?;
    }

    tracing::debug!("Done! End Block: {}", provider.get_block_number().await?);

    Ok(())
}
