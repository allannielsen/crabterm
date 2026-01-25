#!/usr/bin/env bash

set -e
set -x

cargo build --release
sudo cp target/release/crabterm /usr/local/bin/

