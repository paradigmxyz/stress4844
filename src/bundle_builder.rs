use rand::{distributions::Standard, Rng};
#[tracing::instrument(skip_all, name = "construct_bundle")]

/// 1 kilobyte = 1024 bytes
const KB: usize = 1024;

/// Arbitrarily chosen number to cover for nonce+from+to+gas price size in a serialized
/// transaction
const TRIM_BYTES: usize = 500;

async fn construct_bundle<M: Middleware + 'static>(
    provider: Arc<SignerMiddleware<M, LocalWallet>>,
    tx: &TransactionRequest,
    gas_limit: U256,
    fill_pct: u8,
    mut nonce: U256,
    payment: U256,
    chunk_size: U256,
) -> Result<BundleRequest> {
    // `CHUNKS_SIZE` Kilobytes per transaction, shave off 500 bytes to leave room for
    // the other fields to be serialized.
    let chunk = chunk_size * KB - TRIM_BYTES;

    // Sample junk data for the blob.
    let blob = rand::thread_rng()
        .sample_iter(Standard)
        .take(6 * 1024)
        .collect::<Vec<u8>>();

    let gas_per_tx = provider.estimate_gas(&tx.clone().into(), None).await?;
    tracing::debug!("tx cost {} gas", gas_per_tx);

    // For each block, we want `fill_pct` -> we generate N transactions to reach that.
    let gas_used_per_block = gas_limit * fill_pct / 100;

    let max_txs_per_block = (gas_used_per_block / gas_per_tx).as_u64();
    tracing::debug!(max_txs_per_block);

    // TODO: Figure out why making a bundle too big fails.
    let txs_per_block = 10;

    eyre::ensure!(
        max_txs_per_block >= txs_per_block,
        "tried to submit more transactions than can fit in a block"
    );
    let blob_len = tx.data.as_ref().map(|x| x.len()).unwrap_or_default();
    tracing::debug!("submitting {txs_per_block} {blob_len} byte txs per block",);

    let gas_price = provider.get_gas_price().await?;

    // Construct the bundle
    let mut bundle = BundleRequest::new();
    for _ in 0..txs_per_block {
        let mut tx = tx.clone();

        // increment the nonce and apply it
        tx.nonce = Some(nonce);
        nonce += 1.into();
        tx.gas = Some(gas_per_tx);

        // make into typed tx for the signer
        let tx = tx.into();
        let signature = provider.signer().sign_transaction(&tx).await?;
        let rlp = tx.rlp_signed(&signature);
        bundle = bundle.push_transaction(rlp);
    }

    tracing::debug!("signed {} transactions", txs_per_block);

    // let payment = TransactionRequest::new()
    //     .to(COINBASE_PAYER_ADDR.parse::<Address>()?)
    //     .nonce(nonce)
    //     .gas(30000)
    //     .gas_price(gas_price)
    //     .value(payment)
    //     .into();
    // let signature = provider.signer().sign_transaction(&payment).await?;
    // let rlp = tx.rlp_signed(&signature);
    // bundle = bundle.push_transaction(rlp);

    Ok(bundle)
}
