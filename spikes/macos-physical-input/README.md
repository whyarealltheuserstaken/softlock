# m05 physical vs posted input kit

Kit only:

```bash
cargo run --locked --release --manifest-path spikes/macos-physical-input/Cargo.toml -- --kit
```

Do **not** run bare `./tests/hardware/macos/run.sh` for this kit. That is the full hardware suite (black screen, lid, lock).

`--runner` is the GitHub slice: second-process posts only. No arguments prints usage and does not post keys. The tap is listen-only and is released on exit, including Control-C.
