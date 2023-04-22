# Type-Safe Rust Wrappers for various ESP-IDF services

[![CI](https://github.com/esp-rs/esp-idf-svc/actions/workflows/ci.yml/badge.svg)](https://github.com/esp-rs/esp-idf-svc/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/esp-idf-svc.svg)]((https://crates.io/crates/esp-idf-svc))
[![Documentation](https://img.shields.io/badge/docs-esp--rs-brightgreen)](https://esp-rs.github.io/esp-idf-svc/esp_idf_svc/index.html)
[![Matrix](https://img.shields.io/matrix/esp-rs:matrix.org?label=join%20matrix&color=BEC5C9&logo=matrix)](https://matrix.to/#/#esp-rs:matrix.org)
[![Wokwi](https://img.shields.io/endpoint?label=wokwi&url=https%3A%2F%2Fwokwi.com%2Fbadge%2Fsimulate-in-wokwi.json)]((https://wokwi.com/projects/332188235906155092))


- Run ESP-IDF's FreeRTOS using safe Rust code
- Provides wrappers and abstractions for ESP-IDF services like WiFi, networking, HTTP, and logging
- Enables running Rust standard library code on a ESP
- Contains implementations of portable embedded traits defined in the [embedded-svc](https://github.com/ivmarkov/embedded-svc) project

# Getting Starded

To get started quickly, you have two options:

## Option 1: Zero Setup with the Wokwi Online Simulator

You can use the [Wokwi online simulator](https://wokwi.com/projects/332188235906155092) to experiment with this crate.

## Option 2: Use cargo-generate
Please make sure you have installed all [prerequisites](https://github.com/esp-rs/esp-idf-svc#prerequisites) first!
### Generate
```bash
cargo generate --git https://github.com/esp-rs/esp-idf-template
```
### Build
```bash 
cd <your-project-name>
cargo build
```
### Flash + Run + Monitor
```bash
cargo run
```

### Prerequisites

To use this crate, you will need:

1. Install Rust via rustup
2. Install the `cargo-generate` and `ldproxy` tools via cargo
    ```bash
    cargo install cargo-generate ldproxy
    ```
3. (Linux & macOS) Install libuv
    ```bash
    # macOS
    brew install libuv
    # Debian/Ubuntu/etc.
    apt-get install libuv-dev
    # Fedora
    dnf install systemd-devel
    ```
4. Install toolchain
    *  For RISCV-based chips:
        - Have Clang11 and Python 3.7 or greater installed
        - Install the nightly Rust toolchain:
            ```bash
             rustup toolchain install nightly --component rust-src
             ```
    * For XTENSA-based chips:
        - Install `espup` either by building it with Cargo or by downloading the binary from https://github.com/esp-rs/espup/releases
            ```bash
            cargo install espup
            ```
       - Run it
            ```bash
            espup install
            ```

For a comprehansive setup guide check out the [template](https://github.com/esp-rs/esp-idf-template#prerequisites) or the [book](https://esp-rs.github.io/book/)

## Removing the project + toolchain
To remove the project generated using cargo generate, simply delete the directory that was created. For the RISC-V case, no additional cleanup is needed.

For the XTENSA case, you need to remove the XTENSA toolchain installed via espup. You can do this by running `espup uninstall`.

# Chat
Join the ESP-RS community on Matrix chat for help or questions: https://matrix.to/#/#esp-rs:matrix.org
# Aditional Information

* The [Rust on ESP Book](https://esp-rs.github.io/book/)
* The [embedded-svc](https://github.com/esp-rs/embedded-svc) project
* The [esp-idf-template](https://github.com/esp-rs/esp-idf-template) project
* The [esp-idf-sys](https://github.com/esp-rs/esp-idf-sys) project
* The [esp-idf-hal](https://github.com/esp-rs/esp-idf-hal) project
* The [Rust for Xtensa toolchain](https://github.com/esp-rs/rust-build)
* The [Rust-with-STD demo](https://github.com/ivmarkov/rust-esp32-std-demo) project
