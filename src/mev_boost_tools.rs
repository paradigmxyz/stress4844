// ethereum specific
use crate::mev_boost_tools::k256::schnorr::SigningKey;
use ethers::prelude::{signer::SignerMiddleware, *};
use ethers_flashbots::FlashbotsMiddleware;

// general web utils
use std::{sync::Arc, time::Duration};
use url::Url;

pub async fn initialize_mev_boost(
    rpc_url: String,
    tx_signer: String,
    bundle_signer: String,
    interval: Duration,
) -> (
    H160,
    U256,
    u64,
    Arc<
        SignerMiddleware<
            FlashbotsMiddleware<Arc<Provider<Provider>>, Wallet<SigningKey>>,
            Wallet<SigningKey>,
        >,
    >,
) {
    let bundle_signer = bundle_signer.parse::<LocalWallet>()?;

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
    tracing::debug!("block gas limit: {} gas", block.gas_limit);
    let provider =
        Arc::new(SignerMiddleware::new_with_provider_chain(bundle_middleware, signer).await?);
    let chain_id = provider.signer().chain_id();

    return (address, nonce, chain_id, provider);
}
