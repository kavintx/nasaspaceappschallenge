#!/usr/bin/env python3
"""Flame in Freefall — one-command launcher.

Builds and runs the site in Docker, so nothing Rust needs to be installed
(Windows machines with Smart App Control can't run cargo natively at all).

    python main.py                    # the dashboard
    python main.py --backend          # + the research backend (scan, accounts, AI)
    python main.py --stop             # stop what this script started
    python main.py --logs             # follow the dashboard's log

Requires Python 3.12 or newer. What gets built happens inside a
rust:1-bookworm container; the compiled artifacts and the cargo registry
live in named Docker volumes, so the second build takes seconds, not minutes.
"""

import argparse
import contextlib
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent
IMAGE = "rust:1-bookworm"
WEB_CONTAINER = "flame-web"
BACKEND_CONTAINER = "flame-backend"
REGISTRY_VOLUME = "fire-intel-cargo-registry"
WEB_TARGET_VOLUME = "kav-target"
BACKEND_TARGET_VOLUME = "fireset-target"


def docker(args: list[str], check: bool = True, capture: bool = False) -> str:
    """Run one docker command. A plain argv list — no shell, no path mangling."""
    visible = "docker " + " ".join(a for a in args if not a.startswith("--"))
    print(f"  $ {visible if len(visible) < 100 else visible[:97] + '…'}")
    result = subprocess.run(["docker", *args], check=check, capture_output=capture, text=True)
    return result.stdout if capture else ""


def host_path(p: Path) -> str:
    """A path Docker Desktop accepts on every host OS."""
    return str(p).replace("\\", "/")


def ensure_volume(name: str) -> None:
    docker(["volume", "create", name], check=False)


def remove_container(name: str) -> None:
    docker(["rm", "-f", name], check=False)


def env_value(env_path: Path, key: str, default: str | None = None) -> str | None:
    """First non-empty KEY=value in an env file, comments ignored."""
    if not env_path.is_file():
        return default
    for line in env_path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if line and not line.startswith("#") and line.partition("=")[0].strip() == key:
            if value := line.partition("=")[2].strip():
                return value
    return default


def rewrite_env_line(env_path: Path, key: str, new_value: str) -> None:
    """Set KEY=new_value in an env file (first match), keeping everything else."""
    lines = env_path.read_text(encoding="utf-8").splitlines()
    for i, line in enumerate(lines):
        if not line.strip().startswith("#") and line.partition("=")[0].strip() == key:
            lines[i] = f"{key}={new_value}"
            break
    else:
        lines.append(f"{key}={new_value}")
    env_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def ensure_web_env() -> Path:
    env = REPO / "web" / ".env"
    if not env.is_file():
        env.write_text((REPO / "web" / ".env.example").read_text(encoding="utf-8"), encoding="utf-8")
        print("  created web/.env from the example — browse mode works as-is.")
        print("  add API_AUTH_TOKEN (and VISION_PROVIDER=backend) for scanning and accounts.")
    return env


def build_web() -> None:
    ensure_volume(REGISTRY_VOLUME)
    ensure_volume(WEB_TARGET_VOLUME)
    print("Building the dashboard (first run downloads crates and takes a few minutes)…")
    docker([
        "run", "--rm",
        "-e", "CARGO_HOME=/cargo",
        "-v", f"{host_path(REPO)}:/app",
        "-v", f"{WEB_TARGET_VOLUME}:/app/web/target",
        "-v", f"{REGISTRY_VOLUME}:/cargo/registry",
        "-w", "/app/web", IMAGE, "cargo", "build",
    ])


def run_web(port: int) -> None:
    remove_container(WEB_CONTAINER)
    docker([
        "run", "-d",
        "--name", WEB_CONTAINER,
        "--restart", "unless-stopped",
        "--env-file", host_path(REPO / "web" / ".env"),
        "-e", f"PORT={port}",
        "--add-host=host.docker.internal:host-gateway",
        "-p", f"{port}:{port}",
        "-v", f"{host_path(REPO)}:/app",
        "-v", f"{WEB_TARGET_VOLUME}:/app/web/target",
        "-w", "/app/web", IMAGE, "./target/debug/flame_in_freefall",
    ])


def build_backend(fireset: Path) -> None:
    ensure_volume(REGISTRY_VOLUME)
    ensure_volume(BACKEND_TARGET_VOLUME)
    print(f"Building the research backend from {fireset} (first run takes a few minutes)…")
    docker([
        "run", "--rm",
        "-e", "CARGO_HOME=/cargo",
        "-v", f"{host_path(fireset)}:/workspace",
        "-v", f"{BACKEND_TARGET_VOLUME}:/workspace/target",
        "-v", f"{REGISTRY_VOLUME}:/cargo/registry",
        "-w", "/workspace", IMAGE, "cargo", "build",
    ])


def run_backend(fireset: Path, port: int) -> None:
    env = fireset / ".env"
    if not env.is_file():
        sys.exit(
            f"error: {env} not found. Copy {fireset / '.env.example'} to .env there and fill in\n"
            "DATABASE_URL (Supabase Session pooler), API_AUTH_TOKEN (24+ chars) and the model address."
        )
    remove_container(BACKEND_CONTAINER)
    docker([
        "run", "-d",
        "--name", BACKEND_CONTAINER,
        "--restart", "unless-stopped",
        "--env-file", host_path(env),
        "-e", f"BIND_ADDR=0.0.0.0:{port}",
        "--add-host=host.docker.internal:host-gateway",
        "-p", f"{port}:{port}",
        "-v", f"{host_path(fireset)}:/workspace",
        "-v", f"{BACKEND_TARGET_VOLUME}:/workspace/target",
        "-w", "/workspace", IMAGE, "./target/debug/api",
    ])


def wait_up(port: int, what: str, seconds: int = 60) -> bool:
    for _ in range(seconds):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=2):
                return True
        except urllib.error.HTTPError:
            return True  # any HTTP answer means the server is listening
        except OSError:
            time.sleep(1)
    print(f"  {what} did not answer on port {port} within {seconds}s — check `python main.py --logs`.")
    return False


def lan_hint(port: int) -> None:
    # Every non-loopback address this machine answers on — usually the Wi-Fi one
    # first, but adapters vary, so they are all listed for the phone to try.
    with contextlib.suppress(OSError):
        seen = list(dict.fromkeys(a for a in socket.gethostbyname_ex(socket.gethostname())[2]
                                  if not a.startswith("127.")))
        if seen:
            print("Phone on the same Wi-Fi:  " + "  ".join(f"http://{a}:{port}" for a in seen[:3]))
            print("(the camera button then opens the phone's own camera app — browsers")
            print(" only hand out the webcam over HTTPS.)")


def stop_all() -> None:
    remove_container(WEB_CONTAINER)
    remove_container(BACKEND_CONTAINER)
    print(f"Stopped {WEB_CONTAINER} and {BACKEND_CONTAINER} (if they were running).")


def main() -> None:
    ap = argparse.ArgumentParser(description="Build and run Flame in Freefall in Docker.")
    ap.add_argument("--port", type=int, default=None,
                    help="port for the dashboard (default: PORT in web/.env, else 3101)")
    ap.add_argument("--backend", nargs="?", const="", metavar="FIRESET_PATH",
                    help="also build and run the research backend; optional path to a Fireset "
                         "checkout (default: a 'fireset' folder next to this repository)")
    ap.add_argument("--backend-port", type=int, default=8080, help="port for the backend (default 8080)")
    ap.add_argument("--no-build", action="store_true", help="skip cargo build, run existing binaries")
    ap.add_argument("--stop", action="store_true", help="stop the containers this script started")
    ap.add_argument("--logs", action="store_true", help="follow the dashboard's log and exit on Ctrl-C")
    args = ap.parse_args()

    if args.stop:
        stop_all()
        return
    if args.logs:
        subprocess.run(["docker", "logs", "-f", WEB_CONTAINER])
        return

    if shutil.which("docker") is None:
        sys.exit("error: docker is not on PATH. Install Docker Desktop, then run this again.")
    try:
        docker(["ps"], capture=True, check=True)
    except subprocess.CalledProcessError:
        sys.exit("error: Docker is not running. Start Docker Desktop, then run this again.")

    web_env = ensure_web_env()
    port = args.port or int(env_value(web_env, "PORT", "3101"))

    fireset: Path | None = None
    match args.backend:
        case "":
            fireset = REPO.parent / "fireset"
        case None:
            pass
        case other:
            fireset = Path(other).resolve()
    if fireset and not (fireset / "Cargo.toml").is_file():
        sys.exit(
            f"error: no Rust project at {fireset}.\n"
            "Clone it first:  git clone https://github.com/MonsterLXW/Fireset ../fireset\n"
            "…or drop --backend to run the dashboard alone (browsing works, scanning does not)."
        )

    if not args.no_build:
        build_web()
        if fireset:
            build_backend(fireset)

    # The dashboard talks to the backend from inside its own container, where the
    # host is reachable as host.docker.internal. Fix the usual copy-paste default.
    if fireset:
        current = env_value(web_env, "API_BASE_URL", "") or ""
        if not current or "127.0.0.1" in current or "localhost" in current:
            rewrite_env_line(web_env, "API_BASE_URL", f"http://host.docker.internal:{args.backend_port}")
            print(f"  web/.env: API_BASE_URL set to host.docker.internal:{args.backend_port}")

    if fireset:
        run_backend(fireset, args.backend_port)
        wait_up(args.backend_port, "backend")

    run_web(port)
    if wait_up(port, "dashboard"):
        print(f"\n  Flame in Freefall:  http://localhost:{port}")
        if fireset:
            print(f"  research backend:   http://localhost:{args.backend_port}")
        else:
            print("  browsing works; for scanning, accounts and AI answers re-run with:  python main.py --backend")
        lan_hint(port)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as e:
        sys.exit(f"docker failed with exit code {e.returncode} — see the output above.")
