#!/bin/bash

RUSTFLAGS="--cfg loom" cargo test --lib --release $@
