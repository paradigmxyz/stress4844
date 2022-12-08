use rand::{distributions::Standard, Rng};
use std::sync::Arc;

use ethers::prelude::{signer::SignerMiddleware, *};
use ethers_flashbots::BundleRequest;

/// 1 kilobyte = 1024 bytes
const KB: usize = 1024;

/// Arbitrarily chosen number to cover for nonce+from+to+gas price size in a serialized
/// transaction.  TODO: get the actual overhead from the signing, etc. to pack more fully
const TRIM_BYTES: u32 = 500;

#[tracing::instrument(skip_all, name = "construct_bundle")]
fn construct_tx(
    chain_id: u64,
    address: Address,
    receiver: Address,
    data_size: usize,
    gas_price: U256,
) -> ethers::prelude::TransactionRequest {
    // Craft the transaction.
    let blob = generate_random_data(data_size);

    let tx = TransactionRequest::new()
        .chain_id(chain_id)
        .value(0)
        .from(address)
        .to(receiver)
        .data(blob)
        .gas_price(gas_price);

    return tx;
}

fn generate_random_data(size: usize) -> Vec<u8> {
    let blob = rand::thread_rng()
        .sample_iter(Standard)
        .take(size) //6 * 1024)
        .collect::<Vec<u8>>();
    return blob;
}

pub async fn construct_bundle<M: Middleware + 'static>(
    chain_id: u64,
    address: H160,
    receiver: Address,
    provider: Arc<SignerMiddleware<M, LocalWallet>>,
    gas_limit: U256,
    fill_pct: u8,
    mut nonce: U256,
    payment: U256,
    chunk_size: U256,
) -> Result<BundleRequest, eyre::ErrReport> {
    // `CHUNKS_SIZE` Kilobytes per transaction, shave off 500 bytes to leave room for
    // the other fields to be serialized.
    let chunk = chunk_size * KB - TRIM_BYTES;

    // For each block, we want `fill_pct` -> we generate N transactions to reach that.
    let gas_used_per_block = gas_limit * fill_pct / 100;
    let data_size: usize = fill_pct as usize * 2 * 1024 * KB; // block max size is 2MB
    tracing::debug!("data size: {}", data_size);
    let blob = generate_random_data(data_size);

    //let max_txs_per_block = (gas_used_per_block / gas_per_tx).as_u64();
    //tracing::debug!(max_txs_per_block);

    let current_gas_used = TRIM_BYTES;
    // TODO: Figure out why making a bundle too big fails.
    let txs_per_block = 1;

    eyre::ensure!(
        true, //max_txs_per_block >= txs_per_block,
        "tried to submit more transactions than can fit in a block"
    );

    let gas_price = provider.get_gas_price().await?;

    // Construct the bundle
    let mut bundle = BundleRequest::new();

    // fix the true condition to "while space left in the block/bundle"
    while true {
        //for _ in 0..txs_per_block {
        //    let mut tx = tx.clone();
        let mut tx = construct_tx(chain_id, address, receiver, data_size, gas_price);
        let gas_per_tx = provider.estimate_gas(&tx.clone().into(), None).await?;
        tracing::debug!("tx cost {} gas", gas_per_tx);
        let blob_len = tx.data.as_ref().map(|x| x.len()).unwrap_or_default();
        tracing::debug!("submitting {txs_per_block} {blob_len} byte txs per block",);

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
