"""Check architectures and platforms of desktop macOS executables."""
import re
import subprocess


def verify(binary, expected=("arm64", "x86_64")):
    architectures = subprocess.check_output(["lipo", "-archs", str(binary)], text=True).split()
    if set(architectures) != set(expected) or len(architectures) != len(expected):
        raise ValueError(f"Expected macOS architectures {', '.join(expected)}: {architectures}")
    for architecture in architectures:
        build = subprocess.check_output([
            "xcrun", "vtool", "-arch", architecture, "-show-build", str(binary)
        ], text=True)
        platforms = re.findall(r"^\s*platform\s+(\S+)\s*$", build, re.MULTILINE)
        if platforms != ["MACOS"]:
            raise ValueError(f"Expected desktop MACOS platform for {architecture}: {build}")
    return architectures
