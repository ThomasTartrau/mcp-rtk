## [1.6.0](https://gitlab.com/ThomasTartrau/mcp-rtk/compare/v1.5.1...v1.6.0) (2026-04-23)

### Features

* preserve numeric user IDs in condensed user objects ([a32b1f7](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/a32b1f73772e9aad89bf9ca57fe3a99a81ac25cf))

## [1.5.1](https://gitlab.com/ThomasTartrau/mcp-rtk/compare/v1.5.0...v1.5.1) (2026-04-21)

### Bug Fixes

* add SQLite busy timeout and WAL mode for concurrent tracking ([cf39c89](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/cf39c89b8357003d0c9fc69f88b277ea88fa2151))

## [1.5.0](https://gitlab.com/ThomasTartrau/mcp-rtk/compare/v1.4.0...v1.5.0) (2026-04-21)

### Features

* preserve critical context fields in gitlab preset ([df645a2](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/df645a21c50c0def3dfe32fb4f8ded64fe428dcf))
* tune grafana preset for better context preservation ([0257ef4](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/0257ef4532a9979c3c0afd2551c7b16087a3212d))

## [1.4.0](https://gitlab.com/ThomasTartrau/mcp-rtk/compare/v1.3.0...v1.4.0) (2026-03-13)

### Features

* external preset auto-discovery and hot reload ([63b00d4](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/63b00d4cf8341fdfa945b5f1f0d706f5b6140920))

## [1.3.0](https://gitlab.com/ThomasTartrau/mcp-rtk/compare/v1.2.0...v1.3.0) (2026-03-13)

### Features

* add diff, preset init/pull, proptest, benchmarks CI, and security hardening ([87ea88a](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/87ea88ab172d8f4d955da8c132b565d560086b8c))

### Bug Fixes

* add CARGO_HOME/bin to PATH for bencher CLI in CI ([9a3c32e](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/9a3c32e9872882cdb70bdb926dffa3d5e397ed2d))
* conditionally pass --start-point-hash in bench-mr when SHA is available ([54fb388](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/54fb388acb0b0c88661598cbcccc74ce3284d61d))
* remove needs:cargo-check from bench jobs to fix MR pipeline ([ff89208](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/ff89208aff88ac722e06fc1b3283693bb5121440))

## [1.2.0](https://gitlab.com/ThomasTartrau/mcp-rtk/compare/v1.1.0...v1.2.0) (2026-03-13)

### Features

* add install/uninstall commands for MCP config files ([5557426](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/555742636659460ef4013d55838f64dad2daa3aa))

## [1.1.0](https://gitlab.com/ThomasTartrau/mcp-rtk/compare/v1.0.0...v1.1.0) (2026-03-12)

### Features

* add CLI tools, glob patterns, and README ([8752012](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/8752012bfaf7381fa8303337daec183c278e7735))

## 1.0.0 (2026-03-12)

### Features

* initial release — token-optimizing MCP proxy ([5f1525c](https://gitlab.com/ThomasTartrau/mcp-rtk/commit/5f1525c0941fabb55cb99ef6a3fbc52604b48ba7))
