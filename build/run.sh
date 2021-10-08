#!/bin/zsh

geth_pid_file="/tmp/geth.pid"
eth_datadir="/tmp/geth.data"
get_geth_pid() {
  if [[ -s "${geth_pid_file}" ]]; then
    cat "${geth_pid_file}"
  fi
}

get_beefy_pid_file() {
  name=$1
  echo "/tmp/beefy_${name}.pid"
}

get_beefy_process_id() {
  pid_file=$(get_beefy_pid_file "$1")
  if [[ -s "${pid_file}" ]]; then
    cat "${pid_file}"
  fi
}

mode=$1
case ${mode} in
start-geth)
  id=$(get_geth_pid)
  if [ -n "${id}" ]; then
      echo "geth is already running[${id}]"
      exit 0
  fi
  dir=$(dirname "$0")
  geth --datadir "${eth_datadir}" init "${dir}"/t3rn_eth_genesis.json &> /dev/null
  geth --datadir "${eth_datadir}" \
  --vmdebug --networkid 15 \
  --http --http.port 8545 --http.addr 0.0.0.0 --http.api debug,personal,eth,net,web3,txpool \
  --ws --ws.port 8546 --ws.addr 0.0.0.0 --ws.origins "*" --ws.api debug,eth,net,web3 \
  --mine --miner.threads=1 \
  --miner.etherbase=0x87D987206180B8f3807Dd90455606eEa85cdB87a \
  --allow-insecure-unlock \
  --rpc.gascap 100000000 \
  --trace "${eth_datadir}/trace" \
  --gcmode archive &> /tmp/geth.log &
  c_pid=$!
  echo "${c_pid}" > "${geth_pid_file}"
  echo "Started geth[${c_pid}]"
  ;;

stop-geth)
  id=$(get_geth_pid)
  if [ -z "${id}" ]; then
      echo "Geth node not running"
      exit 0
  fi

  kill -9 "${id}" &> /dev/null
  rm -rf /tmp/geth.*
  echo "Geth node stopped"
  ;;

deploy-contracts)
  dir=$(dirname "$0") && \
  source ~/.zshrc && \
  cd "${dir}"/snowbridge/ethereum/ && \
  nvm install 14.16.1 && \
  nvm use 14.16.1 && \
  yarn install && \
  cp env.template .env && \
  npx hardhat deploy --network localhost
  RELAYCHAIN_ENDPOINT="ws://localhost:9944" npx hardhat run ./scripts/configure-beefy.ts --network localhost
  # private key: 0x935b65c833ced92c43ef9de6bff30703d941bd92a2637cb00cfad389f5862109
  # eth_address=0x87D987206180B8f3807Dd90455606eEa85cdB87a
  ;;

clean-contracts)
  echo "Cleaning contracts cache..."
  dir=$(dirname "$0") && \
  cd "${dir}"/snowbridge/ethereum/ && \
  rm -rf .deployments
  echo "done"
  ;;

build-beefy)
  dir=$(dirname "${0}")
  build="${BUILD_BEEFY:-false}"
  if [[ "${build}" == "true" ]]; then
    echo "Building beefy..."
    cargo build --release --manifest-path="${dir}/../beefy/Cargo.toml" &> /tmp/beefy.build
  fi
  ;;

update-chain-spec)
  echo "Updating chain specification with ethereum state"
  dir=$(dirname "${0}")
  "${dir}"/../beefy/target/release/beefy-node build-spec --chain=local --disable-default-bootnode > "${dir}"/local_beefy_spec.json
  source ~/.zshrc && \
  cd "${dir}"/snowbridge/test/ && \
  nvm install 14.16.1 && \
  nvm use 14.16.1 && \
  yarn install
  header=$(curl http://localhost:8545 \
      -X POST \
      -H "Content-Type: application/json" \
      -d '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params": ["latest", false],"id":1}' \
      | node ./scripts/helpers/transformEthHeader.js)

  spec="../../local_beefy_spec.json"
  jq \
      --argjson header "$header" \
      ' .genesis.runtime.ethereumLightClient.initialHeader = $header
      | .genesis.runtime.ethereumLightClient.initialDifficulty = "0x0"
      ' \
      "${spec}" | sponge "${spec}"
  ;;

delete-chain-spec)
  dir=$(dirname "${0}")
  rm -rf "${dir}/local_beefy_spec.json"
  ;;

start-beefy)
  c_pid=$(get_beefy_process_id "$2")
  if [ -n "${c_pid}" ]; then
      echo "Beefy[$2] already running[${c_pid}]"
      exit 0
  fi
  dir=$(dirname "${0}")
  log_file="/tmp/beefy_$2.log"
  chain_spec="${dir}/local_beefy_spec.json"
  "${dir}"/../beefy/target/release/beefy-node --enable-offchain-indexing=true --chain="${chain_spec}" --"$2" --log=main,info --tmp &> "${log_file}" &
  c_pid=$!
  pid_file=$(get_beefy_pid_file "$2")
  echo "${c_pid}" > "${pid_file}"
  echo "Beefy[$2] started[${c_pid}]..."
  ;;

stop-beefy)
  c_pid=$(get_beefy_process_id "$2")
  if [ -z "${c_pid}" ]; then
      echo "Beefy[$2] not running"
      exit 0
  fi

  kill -9 "${c_pid}" &> /dev/null
  pid_file=$(get_beefy_pid_file "$2")
  rm -rf "${pid_file}"
  echo "Beefy[$2] stopped"
  ;;

all)
  echo "Starting environment. may take sometime..."
  $0 start-geth && \
  $0 build-beefy && \
  $0 update-chain-spec && \
  $0 start-beefy alice && \
  $0 start-beefy bob && \
  $0 start-beefy charlie && \
  $0 deploy-contracts && \
  echo "Done"
  ;;

clean)
  echo "Cleaning up..."
  $0 stop-geth && \
  $0 stop-beefy alice && \
  $0 stop-beefy bob && \
  $0 stop-beefy charlie && \
  $0 clean-contracts && \
  $0 delete-chain-spec && \
  echo "Done"
  ;;
esac
