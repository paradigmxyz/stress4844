#!/bin/bash

# fill these values in to setup the expected environment variables in the example commands
# this is optional; the values could also be passed directly into the commands.

export ETH_RPC_URL=ETH_RPC_URL_GOES_HERE
# for example, you can use https://goerli.infura.io/v3/<your infura key>


export SIGNER=TX_SIGNING_KEY_HERE
# required for both mev-boost and mempool submissions
export BUNDLE=MEV_BOOST_KEY_HERE
# required for mev-boost bundles