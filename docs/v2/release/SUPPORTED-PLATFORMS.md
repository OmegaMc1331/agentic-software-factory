# Native qualification scope

| Target | Hosted runner | Required receipts |
| --- | --- | --- |
| Windows x86_64 / MSVC | windows-2025 | Native test, PE, portable ZIP, installation and runtime |
| Linux x86_64 / GNU | ubuntu-24.04 | Native test, ELF and library requirements, portable tarball, installation and runtime |
| macOS ARM64 | macos-15 | Native test, ARM64 Mach-O and package, installation, runtime and applicable signing |
| macOS x86_64 | macos-15-intel | Native test, x86_64 Mach-O and package, installation, runtime and applicable signing |

All four targets need exact-source evidence. WSL2 covers native Linux userspace
and retains separate desktop qualification. Record binary architecture, runtime
libraries, package hashes and signing status for each produced artifact.
Optional agent CLIs, language servers and debug adapters have their own native
requirements. Browser-engine results are separate from native Safari, assistive
technology and physical IME testing.
