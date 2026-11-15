"""Check both slices of a universal desktop macOS executable using Apple's tools."""
import re
import subprocess


def verify(binary):
    architectures = subprocess.check_output(["lipo", "-archs", str(binary)], text=True).split()
    if set(architectures) != {"arm64", "x86_64"} or len(architectures) != 2:
        raise ValueError(f"Expected a universal arm64/x86_64 macOS executable: {architectures}")
    for architecture in architectures:
        build = subprocess.check_output([
            "xcrun", "vtool", "-arch", architecture, "-show-build", str(binary)
        ], text=True)
        platforms = re.findall(r"^\s*platform\s+(\S+)\s*$", build, re.MULTILINE)
        if platforms != ["MACOS"]:
            raise ValueError(f"Expected desktop MACOS platform for {architecture}: {build}")
    return architectures
