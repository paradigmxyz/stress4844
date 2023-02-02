use rand::{distributions::Standard, Rng};

use ethers::prelude::*;
use ethers_flashbots::BundleRequest;

use eyre::Result;

/// 1 kilobyte = 1024 bytes
const KB: usize = 1024;

/// Arbitrarily chosen number to cover for nonce+from+to+gas price size in a serialized
/// transaction.  TODO: get the actual overhead from the signing, etc. to pack more fully
const TRIM_BYTES: usize = 300;

#[tracing::instrument(skip_all, name = "construct_bundle")]
fn construct_tx(
    chain_id: u64,
    address: Address,
    receiver: Address,
    data_size: usize,
    gas_price: U256,
) -> ethers::prelude::TransactionRequest {
    // Craft the transaction.  data_size is in bytes
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

pub async fn get_signed_tx<M: Middleware>(
    chain_id: u64,
    address: H160,
    receiver: H160,
    chunk: usize,
    gas_price: U256,
    provider: M,
    nonce: U256,
) -> Result<Bytes>
where
    M::Error: 'static,
{
    let mut tx = construct_tx(chain_id, address, receiver, chunk, gas_price);
    let gas_per_tx = provider.estimate_gas(&tx.clone().into(), None).await?;
    // tracing::debug!("tx cost {} gas", gas_per_tx);
    // let blob_len = tx.data.as_ref().map(|x| x.len()).unwrap_or_default();

    // apply nonce and tx gas limit
    tx.nonce = Some(nonce);
    tx.gas = Some(gas_per_tx);

    // make into typed tx for the signer
    let tx = tx.into();
    let sender = provider.default_sender().unwrap_or_default();
    let signature = provider.sign_transaction(&tx, sender).await?;
    let rlp = tx.rlp_signed(&signature);

    // println!("{}", serde_json::to_string(&tx)?);
    // ad hoc test: submit directly
    // let tx = provider.send_transaction(tx, None).await?.await?;

    // println!("{}", serde_json::to_string(&tx)?);
    Ok(rlp)
}

fn generate_random_data(size: usize) -> Vec<u8> {
    // size is bytes
    let blob = rand::thread_rng()
        .sample_iter(Standard)
        .take(size)
        .collect::<Vec<u8>>();
    return blob;
}

pub async fn construct_bundle<M: Middleware>(
    chain_id: u64,
    address: H160,
    receiver: Address,
    provider: M,
    gas_limit: U256,
    fill_pct: u8,
    mut nonce: U256,
    chunk_size: usize,
    tip_wei: u64,
) -> Result<BundleRequest>
where
    M::Error: 'static,
{
    // `CHUNKS_SIZE` Kilobytes per transaction, shave off 300 bytes to leave room for
    // the other fields to be serialized.
    let chunk = chunk_size * KB - TRIM_BYTES;

    // For each block, we want `fill_pct` * 2MB of call data.
    // we generate FLOOT(2MB / chunk_size) transactions of size "chunk_size"
    // and then one final "remainder" transaction to reach the desired fill_pct
    let gas_used_per_block = gas_limit * fill_pct / 100;
    let total_data_size: usize = fill_pct as usize * 2 * 1024 * KB / 100; // block max size is 2MB
    tracing::debug!(
        "total data size: {}, gas_used_per_block: {}, blob size (bytes) per tx: {}",
        total_data_size,
        gas_used_per_block,
        chunk
    );

    //let max_txs_per_block = (gas_used_per_block / gas_per_tx).as_u64();
    //tracing::debug!(max_txs_per_block);

    let mut current_data_used = TRIM_BYTES;
    // TODO: Figure out why making a bundle too big fails.
    let txs_per_block = total_data_size / chunk;
    // tracing::debug!("txs per block: {}", txs_per_block);

    let default_gas_price = provider.get_gas_price().await?;

    let gas_price = U256::from(tip_wei) + default_gas_price;
    tracing::debug!("got gas_price {default_gas_price} from provider, increased to {gas_price}");

    // Construct the bundle
    let mut bundle = BundleRequest::new();

    for _ in 0..txs_per_block {
        let rlp = get_signed_tx(
            chain_id, address, receiver, chunk, gas_price, &provider, nonce,
        )
        .await?;
        bundle = bundle.push_transaction(rlp);
        nonce += 1.into();
        current_data_used += chunk;
    }

    // fill the "remainder" of the block with leftover datasize
    let remaining_data = total_data_size - current_data_used - TRIM_BYTES;
    tracing::debug!("signed {txs_per_block} transactions of {chunk} size each, filling remainder {remaining_data}");
    let last_rlp = get_signed_tx(
        chain_id,
        address,
        receiver,
        remaining_data,
        gas_price,
        &provider,
        nonce,
    )
    .await?;
    bundle = bundle.push_transaction(last_rlp);

    // couldn't get this way working, so instead we just overpay on gas
    // in a legacy transaction within the bundle.  the excess gas price is
    // kept by the proposer as the bribe.

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
