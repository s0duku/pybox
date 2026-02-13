#!/usr/bin/env python3
"""
Pre-build script for pybox-reactor.wasm
Builds the WASM module and copies it to python/pybox/image/
"""
import subprocess
import shutil
from pathlib import Path


def check_and_install_target():
    """Check if wasm32-wasip1 target is installed, install if not"""
    print("Checking for wasm32-wasip1 target...")

    result = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        capture_output=True,
        text=True
    )

    if "wasm32-wasip1" not in result.stdout:
        print("wasm32-wasip1 target not found, installing...")
        result = subprocess.run(
            ["rustup", "target", "add", "wasm32-wasip1"],
            check=False
        )
        if result.returncode != 0:
            print("Failed to install wasm32-wasip1 target")
            return False
        print("Successfully installed wasm32-wasip1 target")
    else:
        print("wasm32-wasip1 target is already installed")

    return True


def main():
    workspace_root = Path(__file__).parent

    # 检查并安装 target
    if not check_and_install_target():
        return 1

    print("\nBuilding pybox-reactor.wasm for wasm32-wasip1...")

    # 构建 wasm，显示完整输出
    result = subprocess.run(
        [
            "cargo", "build", "--release",
            "--target", "wasm32-wasip1",
            "--package", "pybox-reactor"
        ],
        cwd=workspace_root,
        check=False
    )

    if result.returncode != 0:
        print("\nFailed to build pybox-reactor.wasm")
        return result.returncode

    # 源文件和目标路径
    src = workspace_root / "target/wasm32-wasip1/release/pybox_reactor.wasm"
    dst = workspace_root / "python/pybox/image/pybox_reactor.wasm"

    # 确保目标目录存在
    dst.parent.mkdir(parents=True, exist_ok=True)

    # 复制文件
    shutil.copy(src, dst)
    print(f"Successfully copied pybox_reactor.wasm to {dst.relative_to(workspace_root)}")

    return 0


if __name__ == "__main__":
    exit(main())
