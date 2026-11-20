# m05 physical vs posted input kit

Run it from the hardware suite:

```bash
./tests/hardware/macos/run.sh
```

Or the kit alone:

```bash
cargo run --locked --release --manifest-path spikes/macos-physical-input/Cargo.toml -- --kit
```

`--runner` is the GitHub slice: second-process posts only. The tap is listen-only and is released on exit, including Control-C.
