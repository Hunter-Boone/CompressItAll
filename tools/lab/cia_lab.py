#!/usr/bin/env python3
"""Run Smidge checks on the ConvertSave TestLab machines (DESIGN.md 7.6).

Thin wrapper around /mnt/nas/SharedFolder2/projects/ConvertSave/TestLab/lab.py and
scripts/sync-checkout.sh; nothing here forks those scripts.

  cia_lab.py status
  cia_lab.py sync   {linux|windows|mac} [--repo PATH]     # git bundle of HEAD -> ~/work/compressitall-<sha>
  cia_lab.py build  {linux|windows|mac}                   # cargo build -p cia-cli --release (and cia-desktop when present)
  cia_lab.py matrix {linux|windows|mac} [--smoke] [--presets smoke|all]
  cia_lab.py test   {linux|windows|mac}                   # cargo test --workspace --locked
  cia_lab.py pull   {linux|windows|mac}                   # copy target/matrix results to TestLab/results/<stamp>-<machine>-smidge
"""
import argparse, datetime, os, pathlib, subprocess, sys

LAB = pathlib.Path("/mnt/nas/SharedFolder2/projects/ConvertSave/TestLab")
HERE = pathlib.Path(__file__).resolve().parents[2]


def sha(repo):
    return subprocess.check_output(["git", "-C", str(repo), "rev-parse", "--short", "HEAD"], text=True).strip()


def ssh(machine, cmd, check=True):
    return subprocess.run([sys.executable, str(LAB / "lab.py"), "ssh", machine, "--", cmd], check=check)


def remote_dir(machine, repo, sha=None):
    s = sha or subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
    home = "C:/Users/tester" if machine == "windows" else "~"
    return f"{home}/work/compressitall-{s}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("action", choices=["status", "sync", "build", "matrix", "test", "pull"])
    ap.add_argument("machine", nargs="?", choices=["linux", "windows", "mac"])
    ap.add_argument("--repo", default=str(HERE))
    ap.add_argument("--smoke", action="store_true")
    ap.add_argument("--presets", default="smoke")
    ap.add_argument("--sha", help="full commit of the synced checkout (default: local HEAD)")
    a = ap.parse_args()
    if a.action == "status":
        return subprocess.call([sys.executable, str(LAB / "lab.py"), "status"])
    if not a.machine:
        ap.error("machine required")
    rd = remote_dir(a.machine, a.repo, a.sha)
    if a.action == "sync":
        return subprocess.call(["bash", str(LAB / "scripts" / "sync-checkout.sh"), a.machine, a.repo, "compressitall"])
    if a.machine == "windows":
        # Tools installed by choco in a previous session are not on the SSH session's PATH yet.
        prefix = "$env:PATH += ';C:\\Program Files\\CMake\\bin;C:\\Python314;C:\\Python314\\Scripts;C:\\Python313;C:\\Python313\\Scripts;C:\\Python312;C:\\Python312\\Scripts;C:\\Program Files\\NASM'; "
        pw = lambda c: f'powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "{prefix}{c}"'
        if a.action == "build":
            return ssh(a.machine, pw(f"cd {rd}; cargo build -p cia-cli --release --locked; if (Test-Path apps/desktop/src-tauri) {{ cargo build -p cia-desktop --locked }}")).returncode
        if a.action == "test":
            return ssh(a.machine, pw(f"cd {rd}; cargo test --workspace --locked")).returncode
        if a.action == "matrix":
            flags = ("--smoke " if a.smoke else "") + f"--presets {a.presets}"
            return ssh(a.machine, pw(f"cd {rd}; python tools/fixtures/generate.py --quick; $env:CIA_FFMPEG='ffmpeg'; ./target/release/cia.exe matrix --fixtures fixtures/synth --out target/matrix {flags}")).returncode
    else:
        if a.action == "build":
            return ssh(a.machine, f"cd {rd} && cargo build -p cia-cli --release --locked && (test -d apps/desktop/src-tauri && cargo build -p cia-desktop --locked || true)").returncode
        if a.action == "test":
            return ssh(a.machine, f"cd {rd} && cargo test --workspace --locked").returncode
        if a.action == "matrix":
            flags = ("--smoke " if a.smoke else "") + f"--presets {a.presets}"
            return ssh(a.machine, f"cd {rd} && python3 tools/fixtures/generate.py --quick && CIA_FFMPEG=$(command -v ffmpeg) ./target/release/cia matrix --fixtures fixtures/synth --out target/matrix {flags}").returncode
    if a.action == "pull":
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        dest = LAB / "results" / f"{stamp}-{a.machine}-smidge"
        dest.mkdir(parents=True, exist_ok=True)
        addr = subprocess.check_output([sys.executable, str(LAB / "lab.py"), "status", a.machine], text=True).split()[-2]
        src = f"tester@{addr}:{rd}/target/matrix/"
        rc = subprocess.call(["scp", "-r", "-o", "StrictHostKeyChecking=yes", src.replace("~", "."), str(dest)])
        print(dest)
        return rc
    return 0


if __name__ == "__main__":
    sys.exit(main())
