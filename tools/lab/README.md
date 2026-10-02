# Lab runs

`cia_lab.py` drives the ConvertSave TestLab machines (Linux and Windows VMs on the Evo host, the Mac mini) for Smidge. It reuses `ConvertSave/TestLab/lab.py` and `scripts/sync-checkout.sh` (which now takes an optional checkout name).

Per-machine dependencies, beyond what ConvertSave already needs (Rust 1.98.1, Node 22, FFmpeg):

| Machine | Extra packages | Why |
|---|---|---|
| Linux VM | `cmake nasm python3-pil` | `audiopus_sys` (libopus) builds with CMake; `mozjpeg-sys` SIMD needs NASM (falls back to C without it); fixtures need Pillow |
| Windows VM | `cmake`, `nasm` (choco), `pip install pillow` | same |
| Mac mini | `brew install cmake nasm`, `pip3 install pillow` | same |

Typical loop:

```
python3 tools/lab/cia_lab.py sync linux
python3 tools/lab/cia_lab.py build linux
python3 tools/lab/cia_lab.py matrix linux --smoke
python3 tools/lab/cia_lab.py pull linux        # -> ConvertSave/TestLab/results/<stamp>-linux-smidge/
```

Results land in the same `TestLab/results` folder the dashboard at http://test-results.floo.network reads.
