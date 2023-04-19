# step 1 query CODE_ID from deployed contract via command
#readonly DEPLOYMENT_TX_HASH="D8203B1130D72A61E7A91FCA2464DD4B28F220AC1EE6703CAACAD88F440EED1C"
#injectived query tx $DEPLOYMENT_TX_HASH --node=https://k8s.testnet.tm.injective.network:443

CODE_ID="712"
INJ_ADDRESS="inj1lsuerzge89tyd4p2pj8wrj903v5ja5emmugntd"


# shellcheck disable=SC2089
INIT='{"admin":"inj1lsuerzge89tyd4p2pj8wrj903v5ja5emmugntd","supported_tokens": [["inj1jyldpwc5ycuj5nn7tg7wejfs62pvxsy0l9n9xs", "inj1hwsuf0n59cm6mdhzd7rn0v79klu4jnugca0nxy"],["inj16d2fzkwzj2z39p6km9x5gk45r729lx5qk3u95t", "inj15h4pkwy9mdcz6hwahcdfuuhwfkl6jwmzerwthw"],["inj10merj58djsdq82xqq30vyvlxjzsan2dswuzfr9", "inj1m52r8r9hh8ut7n84a88vjrqxahyvw76reulxel"],["inj1ee8gx5k0qmq0aywkm6dgaxdqnys2qw8vfajaxn", "inj1qh6zj4j28xtmpqel2el4wjavpef9vmyahzsp0l"]],"pyth_contract_addr": "inj1z60tg0tekdzcasenhuuwq3htjcd5slmgf7gpez"}'


# shellcheck disable=SC2046
yes 12345678 | injectived tx wasm instantiate $CODE_ID $INIT --label="iLend Contract" --from=$(echo $INJ_ADDRESS) --chain-id="injective-888" --yes --gas-prices=500000000inj --gas=20000000 --admin=$(echo $INJ_ADDRESS) --node=https://k8s.testnet.tm.injective.network:443