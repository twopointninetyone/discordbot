## Dependencies
Install Rust  
```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Setup
Edit the .ex_env file accordingly, and copy it into a .env file  
```sh
vim .ex_env && cp .ex_env .env
```

## Startup
Execute the start.sh file  
```sh
./start.sh
```  
This'll create and start up a docker container, compile the Rust code, and hopefully run it
