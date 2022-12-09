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
    bundle_signer_key: String,
    interval: Duration,
) -> Result<
    (
        H160,
        U256,
        Arc<
            SignerMiddleware<
                FlashbotsMiddleware<Arc<Provider<Provider<Http>>>, Wallet<SigningKey>>,
                Wallet<SigningKey>,
            >,
        >,
    ),
    ProviderError,
> {
    let bundle_signer = bundle_signer_key.parse::<LocalWallet>()?;

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

    Ok(address, chain_id, provider)
}
