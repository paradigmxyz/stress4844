// ethereum specific
use ethers_flashbots::{BundleRequest, FlashbotsMiddleware};
use ethers::prelude::{signer::SignerMiddleware, *};

// general web utils
use url::Url;

fn initialize_mev_boost(
    rpc_url: String,
    tx_signer: String
) -> (String, number, FlashbotsMiddleware:: ){
    let bundle_signer = opts
        .bundle_signer
        .strip_prefix("0x")
        .unwrap_or(&opts.bundle_signer)
        .parse::<LocalWallet>()?;

    let provider = Arc::new(Provider::try_from(rpc_url)?.interval(interval));
    let signer = tx_signer.parse::<LocalWallet>()?;

    let bundle_middleware = FlashbotsMiddleware::new(
        provider.clone(),
        Url::parse("https://relay-goerli.flashbots.net/")?,
        bundle_signer,
    );



    let address = signer.address();
    let balance = provider.get_balance(address, None).await?;
    let block = provider
        .get_block(BlockNumber::Latest)
        .await?
        .expect("could not get latest block");
    let nonce = provider
        .get_transaction_count(address, Some(BlockNumber::Pending.into()))
        .await?;

    tracing::info!(
        "starting benchmark from {:?} (balance: {} ETH, nonce: {})",
        address,
        ethers::core::utils::format_units(balance, "eth")?,
        nonce
    );
    tracing::info!("builder payment {}", payment);
    tracing::debug!("block gas limit: {} gas", block.gas_limit);
    let provider =
        Arc::new(SignerMiddleware::new_with_provider_chain(bundle_middleware, signer).await?);
    let chain_id = provider.signer().chain_id();


    (address, chaind_id, provider);
} fn initialize