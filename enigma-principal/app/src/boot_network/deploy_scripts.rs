// general
use failure::Error;
use rustc_hex::FromHex;
use sgx_types::{sgx_enclave_id_t};
use std::thread;
use std::fs::File;
use std::io::prelude::*;
use std::time;
use std::str;
use serde_derive::*;
use serde_json;
use serde_json::{Value};
use url::{Url};
use tokio_core;
// enigma modules 
use enigma_tools_u::web3_utils::w3utils;
use esgx;
//web3
use web3;
use web3::futures::{Future, Stream};
use web3::contract::{Contract, Options};
use web3::types::{Address, U256, Bytes,BlockNumber,FilterBuilder};
use web3::transports::Http;
use web3::Web3;
use web3::contract::tokens::Tokenize;
use web3::api;


#[derive(Serialize, Deserialize, Debug)]
pub struct ScriptDeployConfig {
    pub ENIGMA_CONTRACT_PATH : String,
    pub ENIGMA_TOKEN_CONTRACT_PATH : String,
    pub ACCOUNT_ADDRESS : String,
    pub URL : String
}

impl ScriptDeployConfig{
    pub fn set_accounts_address(&mut self, new_address : String){
        self.ACCOUNT_ADDRESS = new_address;
    }
    pub fn set_ethereum_url(&mut self, new_address : String){
        self.URL = new_address;
    }
}

pub fn load_config(config_path : &str)->ScriptDeployConfig{
           let mut f = File::open(config_path)
        .expect("file not found.");

       let mut contents = String::new();
        f.read_to_string(&mut contents)
            .expect("canno't read file");

       serde_json::from_str(&contents).unwrap()
}
/// TESTING: this function deploys the Enigma and EnigmaToken contract 
/// delegate the config loading (mutation purposes) to the caller 
/// TODO:: merge with deploy_base_contracts()
/// 
pub fn deploy_base_contracts_delegated(eid : sgx_enclave_id_t, 
    config : ScriptDeployConfig,
    url : Option<Url>)->Result<(Contract<Http>,Contract<Http>),Error>{
    // load all config 
        let config : ScriptDeployConfig = config;
        let (e_abi,e_bytecode) = w3utils::load_contract_abi_bytecode(&config.ENIGMA_CONTRACT_PATH.as_str())
            .expect("canot load enigma contract.");
        let (et_abi,et_bytecode) = w3utils::load_contract_abi_bytecode(&config.ENIGMA_TOKEN_CONTRACT_PATH.as_str())
            .expect("cannot load enigma token contract.");
        // connect to ethereum 
        let url = match url {
            Some(u) => u.into_string(),
            None => config.URL,
        };
        let (eloop,w3) = w3utils::connect(&url.as_str()).expect("cannot connect to ethereum");
        // deploy the enigma token 

        let token_contract = deploy_enigma_token_contract
        (
            &w3, 
            config.ACCOUNT_ADDRESS.clone() , 
            et_abi, 
            et_bytecode
        )
        .expect("cannot deploy enigma token.");    

        // deploy the enigma contract
        let token_addr = token_contract.address();//.to_string();
        let signer_addr = get_signing_address(eid).expect("cannot get signer address from sgx");
        let enigma_contract = deploy_enigma_contract
        (
            &w3, 
            config.ACCOUNT_ADDRESS.clone(), 
            e_abi, 
            e_bytecode, 
            token_addr, 
            signer_addr
        )
        .expect("cannot deplot enigma contract");

        Ok((enigma_contract, token_contract))
}
/// TESTING: this function deploys the Enigma and EnigmaToken contract 
/// /// TODO:: merge with deploy_base_contracts_delegated()
pub fn deploy_base_contracts(eid : sgx_enclave_id_t, 
    config_path : &str,
    url : Option<Url>)->Result<(Contract<Http>,Contract<Http>),Error>{

    // load all config 
    let config : ScriptDeployConfig = load_config(config_path);
    let (e_abi,e_bytecode) = w3utils::load_contract_abi_bytecode(&config.ENIGMA_CONTRACT_PATH.as_str())
        .expect("canot load enigma contract.");
    let (et_abi,et_bytecode) = w3utils::load_contract_abi_bytecode(&config.ENIGMA_TOKEN_CONTRACT_PATH.as_str())
        .expect("cannot load enigma token contract.");
    // connect to ethereum 
    let url = match url {
        Some(u) => u.into_string(),
        None => config.URL,
    };
    let (eloop,w3) = w3utils::connect(&url.as_str()).expect("cannot connect to ethereum");
    // deploy the enigma token 

    let token_contract = deploy_enigma_token_contract
    (
        &w3, 
        config.ACCOUNT_ADDRESS.clone() , 
        et_abi, 
        et_bytecode
    )
    .expect("cannot deploy enigma token.");    

    // deploy the enigma contract
    let token_addr = token_contract.address();
    let signer_addr = get_signing_address(eid).expect("cannot get signer address from sgx");
    let enigma_contract = deploy_enigma_contract
    (
        &w3, 
        config.ACCOUNT_ADDRESS.clone(), 
        e_abi, 
        e_bytecode, 
        token_addr, 
        signer_addr
    )
    .expect("cannot deplot enigma contract");

    Ok((enigma_contract, token_contract))
}
/// TESTING: deploy the EnigmaToken contract
pub fn deploy_enigma_token_contract(w3 : &Web3<Http>, 
    deployer : String, 
    abi : String, 
    bytecode : String)->Result<Contract<Http>,Error>{

    let gas_limit = String::from("5999999");
    let confirmations = 0;
    let polling_interval = 1;
    let tx_params = w3utils::DeployParams::new
    (
        deployer, 
        abi, 
        bytecode, 
        gas_limit, 
        polling_interval, 
        confirmations
    );
    let contract = w3utils::deploy_contract(w3, tx_params, ())
        .expect("failed to deploy EnigmaToken contract");
    Ok(contract)
}

/// TESTING: deploy the Enigma contract
pub fn deploy_enigma_contract(w3 : &Web3<Http>, 
    deployer : String, 
    abi : String, 
    bytecode : String, 
    token_addr : Address, 
    signer_addr : String)->Result<Contract<Http>,Error>{

    // generate ctor params 
    let signer_addr: Address = signer_addr
        .parse().expect("unable to parse signer_addr address");
    let input = (token_addr,signer_addr);
    // tx params 
    let gas_limit = String::from("5999999");
    let confirmations = 0;
    let polling_interval = 1;
    let tx_params = w3utils::DeployParams::new
    (
        deployer, 
        abi, 
        bytecode, 
        gas_limit, 
        polling_interval, 
        confirmations
    );
    // deploy 
    let contract = w3utils::deploy_contract(w3, tx_params, input)
        .expect("failed to deploy Enigma contract");
    Ok(contract)
}

/// get the signer addr 
pub fn get_signing_address(eid : sgx_enclave_id_t)->Result<String,Error>{
        let eid = eid;
        let mut signing_address = esgx::equote::get_register_signing_address(eid)?;
        signing_address = signing_address[2..].to_string();
        Ok(signing_address)
}

/// TESTING: deploy the dummy contract 
fn deploy_dummy_miner(w3 : &Web3<Http>, deployer : &String)->Result<Contract<Http>,Error>{
    // contract path 
    let path = "../app/tests/principal_node/contracts/Dummy.json";
    // build deploy params 
    let deployer = deployer.clone();
    let gas_limit = "5999999";
    let poll_interval : u64 = 1;
    let confirmations : usize = 0;
    let (abi,bytecode) = w3utils::load_contract_abi_bytecode(path)?;

    let tx = w3utils::DeployParams::new(
            deployer,
            abi,bytecode,
            gas_limit.to_string(),
            poll_interval,
            confirmations);
    // deploy
    let contract = w3utils::deploy_contract(&w3, tx,())?;
    Ok(contract)
}

/// TESTING: mimic block creation to test the watch blocks method of the principal node 
pub fn forward_blocks(interval : u64, deployer : String, url : String){
    let (eloop,w3) = w3utils::connect(url.as_str()).expect("cannot connect to ethereum network (miner)");
    let contract = deploy_dummy_miner(&w3, &deployer).expect("cannot deploy dummy miner");
    println!("deployed dummy contract at address = {:?}",contract.address() );
    let deployer : Address = deployer
            .parse()
            .expect("unable to parse deployer address");
    loop {
        let gas_limit = String::from("5999999");
        let mut options = Options::default();
        let mut gas : U256 = U256::from_dec_str(&gas_limit).unwrap();
        options.gas = Some(gas);
        //contract.call("mine",(),deployer,options ).wait().expect("error calling mine on miner.");
        let res =  contract.call("mine",(),deployer,options ).wait();
        match res  {
            Ok(res) => println!("\u{2692}" ),
            Err(e) => println!("[-] error mining block =>{:?}",e),
        };
        thread::sleep(time::Duration::from_secs(interval));
    }
}

#[cfg(test)]  
 mod test {
    use super::*;
    use enigma_tools_u::web3_utils;
    use enigma_tools_u::web3_utils::w3utils;
    use boot_network::deploy_scripts;
    use esgx::general::init_enclave_wrapper;
    use std::env;

    /// This function is important to enable testing both on the CI server and local. 
    /// On the CI Side: 
    /// The ethereum network url is being set into env variable 'NODE_URL' and taken from there. 
    /// Anyone can modify it by simply doing $export NODE_URL=<some ethereum node url> and then running the tests.
    /// The default is set to ganache cli "http://localhost:8545"
    fn get_node_url()-> String {
        env::var("NODE_URL").unwrap_or("http://localhost:8545".to_string())
    }
    
    fn connect()->(web3::transports::EventLoopHandle, Web3<Http>,Vec<Address>){
        let uri = get_node_url();
        let (eloop,w3) = w3utils::connect(&uri).unwrap();
        let accounts = w3.eth().accounts().wait().unwrap();
        (eloop,w3, accounts)
    }
    pub fn run_miner(accounts : &Vec<Address> ){
        let deployer : String = w3utils::address_to_string_addr(&accounts[0]);
        let child = thread::spawn(move || {
            let url = get_node_url();
            deploy_scripts::forward_blocks(1,deployer, url.to_string());
        });
    }
    #[test]
    //#[ignore]
    fn test_deploy_enigma_contract_environment(){
        
        // init enclave 
        
        let enclave = match init_enclave_wrapper() {
            Ok(r) => {
                println!("[+] Init Enclave Successful {}!", r.geteid());
                r
            },
            Err(x) => {
                println!("[-] Init Enclave Failed {}!", x.as_str());
                assert_eq!(0,1);
                return;
            },
        };

        let eid = enclave.geteid();
        let (eloop,w3,accounts) = connect();
        let deployer : String = w3utils::address_to_string_addr(&accounts[0]);
        // load the config 
        let deploy_config = "../app/tests/principal_node/contracts/deploy_config.json";
        let mut config = deploy_scripts::load_config(deploy_config);
        // modify to dynamic address
        config.set_accounts_address(deployer);
        config.set_ethereum_url(get_node_url());
        // deploy all contracts.
        let (enigma_contract, enigma_token ) = deploy_scripts::deploy_base_contracts_delegated
        (
            eid, 
            config, 
            None
        )
        .expect("cannot deploy Enigma,EnigmaToken");
    }
 }