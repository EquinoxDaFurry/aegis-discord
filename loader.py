import os
import sys
import shutil
import subprocess
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import URLError, HTTPError

REPO_OWNER = "EquinoxDaFurry"
REPO_NAME = "aegis-discord"
REPO_BRANCH = "main"

BASE_DIR = Path("/home/container")

RUNTIME_DIR = BASE_DIR / "aegis_runtime"
CACHE_DIR = BASE_DIR / "aegis_cache"
DOWNLOAD_DIR = BASE_DIR / "aegis_download"

ENV_FILE = BASE_DIR / ".env"
CONFIG_FILE = BASE_DIR / "config.json"

PYTHON = "/usr/local/bin/python"

GITHUB_ARCHIVE = (
    f"https://github.com/{REPO_OWNER}/{REPO_NAME}"
    f"/archive/refs/heads/{REPO_BRANCH}.zip"
)

def log(message):
    print(f"[Aegis Loader] {message}", flush=True)

def download(url, destination):
    log(f"Downloading: {url}")

    request = Request(
        url,
        headers={
            "User-Agent": "Aegis-Updater/1.0"
        }
    )

    with urlopen(request, timeout=30) as response:
        with open(destination, "wb") as output:
            shutil.copyfileobj(response, output)

    log(f"Downloaded: {destination}")

def remove_directory(path):
    if path.exists():
        log(f"Removing {path}")
        shutil.rmtree(path)

def download_repository():
    remove_directory(DOWNLOAD_DIR)
    DOWNLOAD_DIR.mkdir(parents=True)

    archive = DOWNLOAD_DIR / "aegis.zip"

    download(GITHUB_ARCHIVE, archive)

    log("Extracting repository...")

    shutil.unpack_archive(
        archive,
        DOWNLOAD_DIR,
        format="zip"
    )

    extracted = [
        item
        for item in DOWNLOAD_DIR.iterdir()
        if item.is_dir()
    ]

    if len(extracted) != 1:
        raise RuntimeError(
            "Unexpected GitHub archive structure."
        )

    repository_root = extracted[0]
    staged_runtime = DOWNLOAD_DIR / "runtime"

    shutil.copytree(
        repository_root,
        staged_runtime
    )

    git_dir = staged_runtime / ".git"

    if git_dir.exists():
        remove_directory(git_dir)

    staged_env = staged_runtime / ".env"

    if staged_env.exists():
        staged_env.unlink()

    staged_config = staged_runtime / "config.json"

    if staged_config.exists():
        staged_config.unlink()

    return staged_runtime

def validate_runtime(runtime):
    log("Validating downloaded Aegis runtime...")

    required_files = [
        "bot.py",
        "hash.py",
        "text.py",
        "database.aegis",
        "requirements.txt",
    ]

    for filename in required_files:
        path = runtime / filename

        if not path.is_file():
            raise RuntimeError(
                f"Required file missing: {filename}"
            )

    if not ENV_FILE.is_file():
        raise RuntimeError(
            "/home/container/.env is missing."
        )

    if not CONFIG_FILE.is_file():
        raise RuntimeError(
            "/home/container/config.json is missing."
        )

    log("Compiling Python files...")

    result = subprocess.run(
        [
            PYTHON,
            "-m",
            "compileall",
            "-q",
            str(runtime),
        ],
        capture_output=True,
        text=True
    )

    if result.returncode != 0:
        raise RuntimeError(
            "Python compilation failed:\n"
            + result.stderr
        )

    log("Runtime validation passed.")

def install_requirements(runtime):
    requirements = runtime / "requirements.txt"

    log("Installing Python dependencies...")

    result = subprocess.run(
        [
            PYTHON,
            "-m",
            "pip",
            "install",
            "-U",
            "--prefix",
            str(BASE_DIR / ".local"),
            "-r",
            str(requirements),
        ],
        text=True
    )

    if result.returncode != 0:
        raise RuntimeError(
            "pip dependency installation failed."
        )

    log("Dependencies installed.")

def deploy(runtime):
    log("Preparing deployment...")

    remove_directory(CACHE_DIR)

    if RUNTIME_DIR.exists():
        log("Moving current runtime to cache...")
        RUNTIME_DIR.rename(CACHE_DIR)

    try:
        log("Promoting new runtime...")

        runtime.rename(RUNTIME_DIR)

    except Exception:
        log("Deployment failed. Restoring previous runtime.")

        if CACHE_DIR.exists():
            CACHE_DIR.rename(RUNTIME_DIR)

        raise

    log("New Aegis runtime deployed.")

def copy_configuration():
    log("Copying persistent configuration...")

    shutil.copy2(
        ENV_FILE,
        RUNTIME_DIR / ".env"
    )

    shutil.copy2(
        CONFIG_FILE,
        RUNTIME_DIR / "config.json"
    )

    log("Persistent configuration copied.")

def start_aegis():
    log("Starting Aegis...")

    os.chdir(RUNTIME_DIR)

    os.execv(
        PYTHON,
        [
            PYTHON,
            str(RUNTIME_DIR / "bot.py")
        ]
    )

def rollback():
    log("Attempting rollback...")

    if RUNTIME_DIR.exists():
        remove_directory(RUNTIME_DIR)

    if CACHE_DIR.exists():
        CACHE_DIR.rename(RUNTIME_DIR)

        copy_configuration()

        log("Previous Aegis runtime restored.")

        start_aegis()

    log("No previous runtime is available.")

    raise RuntimeError(
        "Aegis could not be updated and no backup exists."
    )

def main():
    log("========================================")
    log("        AEGIS BOOTSTRAP LOADER")
    log("========================================")

    try:
        staged_runtime = download_repository()

        validate_runtime(staged_runtime)

        install_requirements(staged_runtime)

        deploy(staged_runtime)

        copy_configuration()

        remove_directory(DOWNLOAD_DIR)

        start_aegis()

    except (
        HTTPError,
        URLError,
        TimeoutError,
        ConnectionError,
    ) as error:

        log(f"GitHub download failed: {error}")
        remove_directory(DOWNLOAD_DIR)

        if RUNTIME_DIR.exists():
            start_aegis()

        if CACHE_DIR.exists():
            CACHE_DIR.rename(RUNTIME_DIR)
            copy_configuration()
            start_aegis()

        log("No usable Aegis installation exists.")

        sys.exit(1)

    except Exception as error:

        log(f"Loader error: {error}")

        remove_directory(DOWNLOAD_DIR)

        if RUNTIME_DIR.exists():
            start_aegis()

        if CACHE_DIR.exists():
            rollback()

        sys.exit(1)

if __name__ == "__main__":
    main()
