// CLI
use clap::Parser;
use ethers::prelude::k256::ecdsa::SigningKey;
use eyre::Result;
use serde_json::{json, Value};
use std::{thread, time};
use tracing_subscriber::{filter::EnvFilter, prelude::*};

// Misc
use chrono::prelude::*;
use ethers::prelude::*;
use ethers_flashbots::FlashbotsMiddleware;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

// local utils
mod bundle_builder;

/// command line arguments for running the script
#[derive(Debug, Parser)]
struct Opts {
    /// The number of blocks to run the stress test for
    #[arg(default_value = "1", long)]
    blocks: usize,

    /// What % of the block to fill (0-100).
    #[arg(default_value = "80", long, short, value_parser = clap::value_parser!(u8).range(1..=100))]
    fill_pct: u8,

    /// How much calldata (in kbytes) to send in each individual transaction.
    /// Note that mempool is limited to 128 in geth, and higher values required
    /// special white listing from flashbots relay on goerli.
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
    /// This is used for reputation within mev-boost.
    #[arg(default_value = "", long, short)]
    bundle_signer: String,

    /// default "tip" is 5gwei.  have noticed that on goerli, inclusion seems to be pretty
    /// insensitive to the bribe/tip amount.  
    #[arg(default_value = "5000000000", long)]
    tip_wei: u64,

    /// do we use mev-boost, or submit via the mempool?
    #[arg(default_value = "false", long, num_args = 0)]
    mem_pool: bool,

    /// if using mempool, how many transactions to submit in parallel?  (with appropriate nonce increment)
    #[arg(default_value = "64", long)]
    mempool_txs: usize,
}

fn http_provider(s: &str) -> Result<String, String> {
    if s.starts_with("http://") || s.starts_with("https://") {
        Ok(s.to_string())
    } else {
        Err(format!("URL does not start with http(s): {s}",))
    }
}

/// log mev-boost bundle landing attempts, and whether they succeeded or not
fn get_attempt_json(
    chunk_size: usize,
    tip_wei: u64,
    fill_pct: u8,
    success: bool,
    block_no: U64,
) -> Value {
    let entry = json!({
            "tip_wei": tip_wei,
            "fill_pct": fill_pct,
            "success": success,
            "time": Utc::now().to_string(),
            "chunk_size": chunk_size,
            "block_no": block_no,
    });
    return entry;
}

fn log_attempt(chunk_size: usize, tip_wei: u64, fill_pct: u8, success: bool, block_no: U64) {
    let _entry = get_attempt_json(chunk_size, tip_wei, fill_pct, success, block_no);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("stress-4844-attempts.json")
        .unwrap();

    let _res = file.write_all(b"\n");
    let res = serde_json::to_writer(file, &_entry);

    match res {
        Err(e) => eprintln!("Couldn't write to file: {}", e),
        Ok(_) => {
            return;
        }
    }
}

/// log individual mempool transactions as they land
///
///
fn get_txn_json(txn: TransactionReceipt) -> Value {
    let entry = json!({
            "gas_price": txn.effective_gas_price,
            "time": Utc::now().to_string(),
            "block_no": txn.block_number.unwrap(),
            "status": txn.status.unwrap(),
    });
    return entry;
}

fn log_txn(txn: TransactionReceipt) {
    let _entry = get_txn_json(txn);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("stress-4844-mempool-txns.json")
        .unwrap();

    let _res = file.write_all(b"\n");
    let res = serde_json::to_writer(file, &_entry);

    match res {
        Err(e) => eprintln!("Couldn't write to file: {}", e),
        Ok(_) => {
            return;
        }
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

    let rpc_url = opts.rpc_url;

    let use_mempool = opts.mem_pool;

    let tx_signer = opts.tx_signer.strip_prefix("0x").unwrap_or(&opts.tx_signer);
    let signer = tx_signer.parse::<LocalWallet>()?;

    let fill_pct = opts.fill_pct; // how much of the full 2MB payload to take up with calldata
    let tip_wei = opts.tip_wei; // how much to overpay on gas, in wei.
    let chunk_size = opts.chunk_size;

    let interval = Duration::from_secs(1);

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::new("stress4844=trace"))
        .init();

    // the "usual" rpc provider, no flashbots mev-boost middleware
    let provider: Arc<Provider<Http>> =
        Arc::new(Provider::<Http>::try_from(rpc_url)?.interval(interval));

    let chain_id = provider.get_chainid().await?.as_u64();

    let address = signer.address();
    let balance = provider.get_balance(address, None).await?;

    tracing::info!(
        "starting benchmark from {:?} (balance: {} ETH)",
        address,
        ethers::core::utils::format_units(balance, "eth")?,
    );

    let mut nonce = provider
        .get_transaction_count(address, Some(BlockNumber::Pending.into()))
        .await?;
    tracing::debug!("current nonce: {nonce}, use_mempool = {use_mempool}");
    // TODO: Do we want this to be different per transaction?
    let receiver: Address = "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".parse()?;

    let block = provider
        .get_block(BlockNumber::Latest)
        .await?
        .expect("could not get latest block");

    if use_mempool {
        let mempool_txs = opts.mempool_txs;
        // Sign transactions with a private key
        let provider = SignerMiddleware::new(provider, signer);
        submit_txns(
            provider,
            chain_id,
            address,
            receiver,
            &mut nonce,
            chunk_size,
            mempool_txs,
        )
        .await?;
    } else {
        let blocks_to_land = opts.blocks;
        let bundle_signer = opts
            .bundle_signer
            .strip_prefix("0x")
            .unwrap_or(&opts.bundle_signer);

        let bundle_signer = bundle_signer.parse::<LocalWallet>()?;

        submit_bundles(
            provider,
            chain_id,
            address,
            receiver,
            &mut nonce,
            block,
            blocks_to_land,
            chunk_size,
            fill_pct,
            tip_wei,
            tx_signer,
            bundle_signer,
        )
        .await?;
    }
    Ok(())
}

/// go through the mempool, for transactions with <= 128kb of calldata each
async fn submit_txns(
    provider: SignerMiddleware<Arc<Provider<Http>>, Wallet<SigningKey>>,
    chain_id: u64,
    address: H160,
    receiver: H160,
    nonce: &mut U256,
    chunk_size: usize,

    mempool_txs: usize,
) -> eyre::Result<()> {
    let mut landed = 0;
    let calldata_bytes = bundle_builder::calldata_kb_to_bytes(chunk_size);

    let default_gas_price = provider.get_gas_price().await?;

    let mut transactions: Vec<Bytes> = Vec::new();

    for i in 0..mempool_txs - 1 {
        let new_nonce = *nonce + U256::from(i);
        let tx = bundle_builder::get_signed_tx(
            chain_id,
            address,
            receiver,
            calldata_bytes,
            default_gas_price,
            provider.clone(),
            new_nonce, //*nonce,
        )
        .await?;
        transactions.push(tx);
    }
    tracing::debug!("generated {mempool_txs} transactions");

    let mut responses = Vec::new();
    for txn in transactions {
        let res = provider.send_raw_transaction(txn);

        responses.push(res);
    }
    let pending_txs = futures::future::try_join_all(responses).await?;
    let receipts: Vec<Option<TransactionReceipt>> =
        futures::future::try_join_all(pending_txs).await?;

    tracing::debug!("submitted {mempool_txs} transactions");

    for receipt in receipts {
        thread::sleep(time::Duration::from_millis(1000));
        if let Some(receipt) = receipt {
            // not hitting this should be rare - somehow get dropped from mempool if gas too low
            landed += 1;
            tracing::info!(
                "{} {landed} on {}",
                receipt.transaction_hash,
                receipt.block_number.unwrap()
            );
            log_txn(receipt);
        }
    }

    Ok(())
}

/// go through mev-boost via flashbots relay, potentially for larger calldata txns
async fn submit_bundles(
    provider: Arc<Provider<Http>>,
    chain_id: u64,
    address: H160,
    receiver: H160,
    nonce: &mut U256,
    block: Block<H256>,
    blocks_to_land: usize,
    chunk_size: usize,
    fill_pct: u8,
    tip_wei: u64,
    tx_signer: &str,
    bundle_signer: Wallet<SigningKey>,
) -> eyre::Result<()> {
    let mut landed = 0;

    let signer = tx_signer.parse::<LocalWallet>()?;

    let bundle_middleware = FlashbotsMiddleware::new(
        provider.clone(),
        Url::parse("https://relay-goerli.flashbots.net/")?, // TODO: make configurable
        bundle_signer,
    );

    let provider =
        Arc::new(SignerMiddleware::new_with_provider_chain(bundle_middleware, signer).await?);

    let mut bundle = bundle_builder::construct_bundle(
        chain_id,
        address,
        receiver,
        provider.clone(),
        block.gas_limit,
        fill_pct,
        *nonce,
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

        let future_block_distance = 1; // 1 by default to get next block
        let target_block = block_number + future_block_distance;
        bundle = bundle
            .set_block(target_block)
            //.set_block(block_number + 1)
            .set_simulation_block(block_number)
            .set_simulation_timestamp(0);

        tracing::debug!(
            "bundle target block {:?}",
            target_block //block_number + FUTURE_BLOCK_DISTANCE
        );

        let pending_bundle = provider.inner().send_bundle(&bundle).await?;
        match pending_bundle.await {
            Ok(bundle_hash) => {
                // TODO: Can we log more info from the Flashbots API?
                tracing::info!("bundle #{} included! hash: {:?}", landed, bundle_hash);

                landed += 1; // actually check if we landed it?
                log_attempt(chunk_size, tip_wei, fill_pct, true, block_number);
            }
            Err(err) => {
                tracing::error!("{}. did not land bundle, retrying.", err);
                log_attempt(chunk_size, tip_wei, fill_pct, false, block_number);
            }
        }
        *nonce = provider
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
            *nonce,
            chunk_size,
            tip_wei,
        )
        .await?;
    }

    tracing::debug!("Done! End Block: {}", provider.get_block_number().await?);

    Ok(())
}
