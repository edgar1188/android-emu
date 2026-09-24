# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

The spice-client crate is a pure Rust SPICE protocol implementation with WebAssembly support, part of the Quickemu Manager project. It provides native and web-based remote desktop connectivity to SPICE-enabled virtual machines.

## Architecture

### Core Components
- `src/client.rs` - Main SPICE client orchestrating all channels
- `src/channels/` - Channel implementations (main, display, inputs, cursor)
- `src/protocol.rs` - SPICE protocol definitions and message structures
- `src/transport/` - Network transports (TCP for native, WebSocket for WASM)
- `src/multimedia/` - Platform-specific backends (GTK4, WASM canvas)
- `src/video/` - Video decoding and frame processing

### Platform Support
- **Native**: Uses Tokio for async runtime, TCP sockets
- **WebAssembly**: Uses wasm-bindgen, WebSocket transport, browser canvas rendering
- **GTK4**: Optional native GUI backend with GStreamer support

## Build and Development Commands

```bash
# Build
cargo build                          # Native debug build
cargo build --release               # Native release build
cargo build --features backend-gtk4 # Build with GTK4 support
./build-wasm.sh                     # Build WASM package (outputs to pkg/)

# Testing
cargo test                           # Run all unit tests
cargo test --features test-utils    # Run with test utilities
just test-unit                      # Unit tests only
just test-integration               # Integration tests

# E2E Testing (Native client with Docker server - fast AIO)

./run-e2e-tests.sh                              # Quick test (auto-starts Docker debug server)
./run-e2e-tests.sh basic qemu                   # Test with QEMU Ubuntu VM (realistic)
./run-e2e-tests.sh basic none --host 192.168.1.100 --port 5900  # Test with existing server
./run-e2e-tests.sh basic system                 # Test with local SPICE server
./run-e2e-tests.sh all                          # Test all implementations
./run-e2e-tests.sh --help                       # Show all options
./setup-test-server.sh                          # Build SPICE test server locally

# Run binaries
cargo run --bin spice-test-client -- --host localhost --port 5900
cargo run --bin spice-e2e-test -- --host localhost --port 5900 --duration 30
cargo run --bin rusty-spice-gtk -- --host localhost --port 5900  # Requires backend-gtk4

# Development tools
cargo fmt                            # Format code
cargo clippy                         # Run linter
RUST_LOG=debug cargo run            # Run with debug logging
```

## Key Implementation Details

### Channel Connection Flow
1. Connect to SPICE server via TCP/WebSocket
2. Perform RedLinkMess handshake on main channel
3. Authenticate with ticket (if required)
4. Initialize additional channels (display, inputs, cursor)
5. Each channel maintains its own connection with unique channel_id

### Message Processing Pattern
```rust
// All channels follow this pattern:
1. Read message header (SpiceDataHeader)
2. Parse message body based on type
3. Handle message in channel-specific way
4. Send acknowledgments when required
```

### Platform-Specific Code
- Use `#[cfg(target_arch = "wasm32")]` for WASM-specific code
- Use `#[cfg(not(target_arch = "wasm32"))]` for native code
- Transport layer abstracts TCP vs WebSocket differences
- Multimedia backends handle platform rendering

### Testing Approach
- Unit tests are in-file with the implementation
- E2E tests verify full client functionality with native client + Docker server (best of both worlds)
- Mock servers in `src/test_utils.rs` for isolated testing

**E2E Testing**:
- Native Rust client for fast builds (no Docker client overhead)
- Docker server for reproducible test environment
- All-in-one script that auto-starts server
- Fast iteration during development

### Common Development Tasks

When implementing new SPICE protocol features:
1. Add message definitions to `src/protocol.rs`
2. Implement channel handler in `src/channels/`
3. Add corresponding multimedia backend support
4. Write unit tests alongside implementation
5. Add E2E test coverage

When debugging SPICE connections:
- Enable `RUST_LOG=debug` for detailed protocol traces
- Use `--host` and `--port` flags for test binaries
- Check Docker server logs: `docker logs <container-id>`
- Use `--trace-on-failure` to save protocol traces
- Wireshark with SPICE dissector for protocol analysis

### Error Handling
- Use `thiserror` for error types in `src/error.rs`
- Propagate errors with `?` operator
- Add context with `.context()` for debugging
- Channel errors should not crash the entire client

### WASM Considerations
- No threading - use async/await patterns
- WebSocket transport required (TCP not available)
- Canvas rendering via web-sys bindings
- Console logging via `tracing-wasm`
- Build with `wasm-pack` for web deployment