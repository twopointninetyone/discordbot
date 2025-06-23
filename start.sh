#!/bin/bash

cleanup() {
  echo "Doing cleanup"
  docker-compose down
}

trap cleanup EXIT

### begin

docker-compose up -d

i=0

while [ $i -lt 10 ] 
do 
  mariadb-admin ping -h 127.0.0.1 -u osaka -posaka
  if [ $? -eq 0 ]; then
    clear
    break
  fi

  sleep 1
  i=$((i + 1))
done

if [ $i -eq 10 ]; then
  echo "Docker container didn't start correctly"
  return 1
fi

if [ ! -d "target/release" ]; then
  cargo build --release
fi

./target/release/aibot
