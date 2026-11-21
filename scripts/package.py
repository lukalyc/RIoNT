#!/usr/bin/env python3
"""Package and verify a RIONT release archive across Windows, Linux, and macOS.

Creates the platform archive (`riont-<version>-<triple>.zip` on Windows using
Python's zipfile module, `.tar.gz` on Unix using tarfile), then immediately
validates the generated archive:
  1. Magic bytes, archive integrity, and Windows native Expand-Archive check.
  2. Exact expected member layout (`riont-<version>/...`) and non-empty sizes.
  3. Binary format, architecture, and (optionally) static Windows CRT linkage.
  4. Execution from an empty temp directory (`--version` and `--help`).
"""
from __future__ import annotations

import argparse
import hashlib
import os
import pathlib
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOC_FILES = ("README.md", "CHANGELOG.md", "LICENSE")
FORBIDDEN_CRT_PREFIXES = ("vcruntime", "msvcp", "ucrtbase", "api-ms-win-crt-")


def read_cargo_version() -> str:
    cargo_toml = ROOT / "Cargo.toml"
    text = cargo_toml.read_text(encoding="utf-8")
    m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not m:
        raise RuntimeError(f"Could not parse version from {cargo_toml}")
    return m.group(1)


def archive_name(version: str, target: str) -> str:
    name_triple = target.replace("-unknown-", "-")
    ext = "zip" if "windows" in target else "tar.gz"
    return f"riont-{version}-{name_triple}.{ext}"


def binary_filename(target: str) -> str:
    return "riont.exe" if "windows" in target else "riont"


def sha256_file(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def create_archive(
    version: str,
    target: str,
    binary_path: pathlib.Path,
    out_dir: pathlib.Path,
) -> pathlib.Path:
    if not binary_path.is_file():
        raise FileNotFoundError(f"Binary not found: {binary_path}")

    out_dir.mkdir(parents=True, exist_ok=True)
    arc_path = out_dir / archive_name(version, target)
    root_dir = f"riont-{version}"
    bin_name = binary_filename(target)

    for doc in DOC_FILES:
        doc_path = ROOT / doc
        if not doc_path.is_file():
            raise FileNotFoundError(f"Required release doc missing: {doc_path}")

    if arc_path.name.endswith(".zip"):
        with zipfile.ZipFile(arc_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
            zf.write(binary_path, arcname=f"{root_dir}/{bin_name}")
            for doc in DOC_FILES:
                zf.write(ROOT / doc, arcname=f"{root_dir}/{doc}")
    else:
        with tarfile.open(arc_path, "w:gz") as tf:
            dir_info = tarfile.TarInfo(name=root_dir)
            dir_info.type = tarfile.DIRTYPE
            dir_info.mode = 0o755
            tf.addfile(dir_info)

            tf.add(binary_path, arcname=f"{root_dir}/{bin_name}", filter=_set_mode(0o755))
            for doc in DOC_FILES:
                tf.add(ROOT / doc, arcname=f"{root_dir}/{doc}", filter=_set_mode(0o644))

    return arc_path


def _set_mode(mode: int):
    def _filter(ti: tarfile.TarInfo) -> tarfile.TarInfo:
        ti.mode = mode
        return ti

    return _filter


def _parse_pe_imports(data: bytes) -> list[str]:
    if len(data) < 0x40 or data[:2] != b"MZ":
        raise ValueError("Windows binary missing MZ header")
    pe_off = struct.unpack_from("<I", data, 0x3C)[0]
    if pe_off + 24 > len(data) or data[pe_off : pe_off + 4] != b"PE\x00\x00":
        raise ValueError("Windows binary missing PE\\0\\0 signature")

    machine = struct.unpack_from("<H", data, pe_off + 4)[0]
    num_sec = struct.unpack_from("<H", data, pe_off + 6)[0]
    opt_size = struct.unpack_from("<H", data, pe_off + 20)[0]
    if machine != 0x8664:
        raise ValueError(f"Expected x86_64 PE machine 0x8664, got 0x{machine:x}")

    opt_off = pe_off + 24
    if opt_off + opt_size > len(data) or opt_size < 112:
        raise ValueError("Truncated PE optional header")
    opt_magic = struct.unpack_from("<H", data, opt_off)[0]
    if opt_magic != 0x20B:
        raise ValueError(f"Expected PE32+ magic 0x20b, got 0x{opt_magic:x}")

    subsystem = struct.unpack_from("<H", data, opt_off + 68)[0]
    if subsystem != 3:
        raise ValueError(f"Expected console subsystem (3), got {subsystem}")

    sec_off = opt_off + opt_size
    sections = []
    for i in range(num_sec):
        so = sec_off + i * 40
        if so + 40 > len(data):
            raise ValueError("Truncated PE section table")
        vsize, vaddr, rawsize, rawptr = struct.unpack_from("<IIII", data, so + 8)
        sections.append((vaddr, vsize, rawptr, rawsize))

    def rva_to_off(rva: int) -> int | None:
        if rva == 0:
            return None
        for va, vs, rp, rs in sections:
            span = max(vs, rs)
            if va <= rva < va + span:
                off = rp + (rva - va)
                if rp != 0 and (rva - va) < rs and off < len(data):
                    return off
        return None

    def read_cstr(rva: int) -> str | None:
        off = rva_to_off(rva)
        if off is None:
            return None
        end = data.find(b"\x00", off)
        if end < 0:
            end = len(data)
        return data[off:end].decode("latin1", errors="replace")

    num_rva = struct.unpack_from("<I", data, opt_off + 108)[0]
    dlls: list[str] = []

    if num_rva > 1:
        imp_rva, _imp_size = struct.unpack_from("<II", data, opt_off + 112 + 8)
        imp_off = rva_to_off(imp_rva)
        while imp_off is not None and imp_off + 20 <= len(data):
            orig_ft, _ts, _fwd, name_rva, ft = struct.unpack_from("<IIIII", data, imp_off)
            if orig_ft == 0 and name_rva == 0 and ft == 0:
                break
            dll = read_cstr(name_rva)
            if dll:
                dlls.append(dll)
            imp_off += 20

    if num_rva > 13:
        delay_rva, _delay_size = struct.unpack_from("<II", data, opt_off + 112 + 13 * 8)
        delay_off = rva_to_off(delay_rva)
        while delay_off is not None and delay_off + 32 <= len(data):
            attrs, name_rva, hmod, iat, int_rva, biat, uiat, ts = struct.unpack_from(
                "<IIIIIIII", data, delay_off
            )
            if attrs == 0 and name_rva == 0 and hmod == 0 and iat == 0:
                break
            if attrs & 1:
                dll = read_cstr(name_rva)
                if dll:
                    dlls.append(dll)
            delay_off += 32

    return dlls


def verify_binary_bytes(data: bytes, target: str, require_static_crt: bool) -> None:
    if target == "x86_64-pc-windows-msvc":
        dlls = _parse_pe_imports(data)
        if require_static_crt:
            bad = [
                d for d in dlls if d.lower().startswith(FORBIDDEN_CRT_PREFIXES)
            ]
            if bad:
                raise AssertionError(
                    f"Windows binary dynamically links CRT DLLs ({bad}); "
                    "build with RUSTFLAGS='-C target-feature=+crt-static'"
                )
    elif target == "x86_64-unknown-linux-gnu":
        if len(data) < 20 or data[:4] != b"\x7fELF":
            raise AssertionError("Linux binary missing ELF magic")
        if data[4] != 2 or data[5] != 1:
            raise AssertionError(
                f"Expected 64-bit LE ELF (class=2, data=1), got ({data[4]}, {data[5]})"
            )
        e_machine = struct.unpack_from("<H", data, 18)[0]
        if e_machine != 0x3E:
            raise AssertionError(
                f"Expected ELF e_machine 0x3e (x86_64), got 0x{e_machine:x}"
            )
    elif target in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
        if len(data) < 16:
            raise AssertionError("Mach-O binary too short")
        magic, cputype, _cpusub, filetype = struct.unpack_from("<IIII", data, 0)
        if magic != 0xFEEDFACF:
            raise AssertionError(
                f"Expected 64-bit LE Mach-O magic 0xfeedfacf, got 0x{magic:x}"
            )
        expected_cpu = 0x0100000C if target.startswith("aarch64-") else 0x01000007
        if cputype != expected_cpu:
            raise AssertionError(
                f"Expected Mach-O cputype 0x{expected_cpu:x}, got 0x{cputype:x}"
            )
        if filetype != 2:
            raise AssertionError(f"Expected Mach-O MH_EXECUTE (2), got {filetype}")
    else:
        raise ValueError(f"Unsupported target triple: {target}")


def verify_archive(
    arc_path: pathlib.Path,
    version: str,
    target: str,
    require_static_crt: bool,
    skip_exec: bool,
) -> None:
    if not arc_path.is_file():
        raise FileNotFoundError(f"Archive not found: {arc_path}")

    root_dir = f"riont-{version}"
    bin_name = binary_filename(target)
    expected_files = {
        f"{root_dir}/{bin_name}",
        f"{root_dir}/README.md",
        f"{root_dir}/CHANGELOG.md",
        f"{root_dir}/LICENSE",
    }

    head = arc_path.read_bytes()[:4]
    with tempfile.TemporaryDirectory(prefix="riont-verify-") as tmp:
        tmp_dir = pathlib.Path(tmp)
        extract_dir = tmp_dir / "extract"
        extract_dir.mkdir()

        if arc_path.name.endswith(".zip"):
            if head != b"PK\x03\x04":
                raise AssertionError(
                    f"{arc_path.name} is not a ZIP archive (magic={head!r})"
                )
            with zipfile.ZipFile(arc_path, "r") as zf:
                bad_member = zf.testzip()
                if bad_member is not None:
                    raise AssertionError(f"Corrupt ZIP entry: {bad_member}")
                files = {}
                for info in zf.infolist():
                    if info.is_dir():
                        continue
                    files[info.filename] = info.file_size
                if set(files) != expected_files:
                    raise AssertionError(
                        f"Unexpected ZIP members: got {sorted(files)}, expected {sorted(expected_files)}"
                    )
                for name, size in files.items():
                    if size <= 0:
                        raise AssertionError(f"Empty ZIP member: {name}")
                zf.extractall(extract_dir)

            if os.name == "nt":
                ps_dir = tmp_dir / "ps-expand"
                ps_dir.mkdir()
                cmd = (
                    f"Expand-Archive -Path '{str(arc_path).replace(chr(39), chr(39)*2)}' "
                    f"-DestinationPath '{str(ps_dir).replace(chr(39), chr(39)*2)}' -Force"
                )
                res = subprocess.run(
                    ["powershell", "-NoProfile", "-Command", cmd],
                    capture_output=True,
                    text=True,
                )
                if res.returncode != 0:
                    raise AssertionError(
                        f"PowerShell Expand-Archive failed: {res.stderr or res.stdout}"
                    )
                ps_bin = ps_dir / root_dir / bin_name
                if not ps_bin.is_file() or ps_bin.stat().st_size == 0:
                    raise AssertionError(
                        f"Expand-Archive did not extract {root_dir}/{bin_name}"
                    )
        elif arc_path.name.endswith(".tar.gz"):
            if head[:2] != b"\x1f\x8b":
                raise AssertionError(
                    f"{arc_path.name} is not a gzip tar archive (magic={head!r})"
                )
            with tarfile.open(arc_path, "r:gz") as tf:
                files = {}
                bin_member = None
                for m in tf.getmembers():
                    if m.isdir():
                        continue
                    if not m.isfile():
                        raise AssertionError(
                            f"Unexpected non-regular member in tar.gz: {m.name}"
                        )
                    files[m.name] = m.size
                    if m.name == f"{root_dir}/{bin_name}":
                        bin_member = m
                if set(files) != expected_files:
                    raise AssertionError(
                        f"Unexpected tar.gz members: got {sorted(files)}, expected {sorted(expected_files)}"
                    )
                for name, size in files.items():
                    if size <= 0:
                        raise AssertionError(f"Empty tar.gz member: {name}")
                if bin_member is None or (bin_member.mode & 0o111) == 0:
                    mode_str = oct(bin_member.mode) if bin_member else "missing"
                    raise AssertionError(
                        f"Binary {root_dir}/{bin_name} is not executable (mode={mode_str})"
                    )
                try:
                    tf.extractall(extract_dir, filter="data")
                except TypeError:
                    tf.extractall(extract_dir)
        else:
            raise ValueError(f"Unsupported archive extension: {arc_path}")

        extracted_bin = extract_dir / root_dir / bin_name
        if not extracted_bin.is_file():
            raise AssertionError(f"Extracted binary missing: {extracted_bin}")
        verify_binary_bytes(extracted_bin.read_bytes(), target, require_static_crt)

        if not skip_exec:
            run_cwd = tmp_dir / "empty-cwd"
            run_cwd.mkdir()
            ver_res = subprocess.run(
                [str(extracted_bin), "--version"],
                cwd=run_cwd,
                capture_output=True,
                text=True,
                timeout=15,
            )
            if ver_res.returncode != 0:
                raise AssertionError(
                    f"--version failed (rc={ver_res.returncode}): {ver_res.stderr}"
                )
            if ver_res.stdout.strip() != f"riont {version}":
                raise AssertionError(
                    f"Expected '--version' output 'riont {version}', got {ver_res.stdout!r}"
                )

            help_res = subprocess.run(
                [str(extracted_bin), "--help"],
                cwd=run_cwd,
                capture_output=True,
                text=True,
                timeout=15,
            )
            if help_res.returncode != 0 or "RIONT" not in help_res.stdout:
                raise AssertionError(
                    f"--help check failed (rc={help_res.returncode}): {help_res.stdout!r} {help_res.stderr!r}"
                )


def main() -> int:
    parser = argparse.ArgumentParser(description="Package and verify a RIONT release build.")
    parser.add_argument("--version", required=True, help="Release version (X.Y.Z, without v)")
    parser.add_argument("--target", required=True, help="Rust target triple")
    parser.add_argument("--binary", help="Path to built riont binary")
    parser.add_argument("--out-dir", help="Output directory for the archive")
    parser.add_argument(
        "--require-static-crt",
        action="store_true",
        help="Fail if a Windows binary imports dynamic MSVC/UCRT runtime DLLs",
    )
    parser.add_argument(
        "--skip-exec",
        action="store_true",
        help="Skip executing the extracted binary (for cross-compiled builds)",
    )
    parser.add_argument(
        "--skip-cargo-version-check",
        action="store_true",
        help="Do not require --version to match Cargo.toml",
    )
    parser.add_argument(
        "--verify-only",
        help="Verify an existing archive file instead of packaging a new one",
    )
    args = parser.parse_args()

    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", args.version):
        print(f"ERROR: invalid version {args.version!r} (expected X.Y.Z)", file=sys.stderr)
        return 2

    if not args.skip_cargo_version_check:
        cargo_ver = read_cargo_version()
        if args.version != cargo_ver:
            print(
                f"ERROR: --version {args.version} does not match Cargo.toml {cargo_ver}",
                file=sys.stderr,
            )
            return 2

    if args.verify_only:
        arc_path = pathlib.Path(args.verify_only).resolve()
    else:
        if not args.binary or not args.out_dir:
            parser.error("--binary and --out-dir are required unless --verify-only is set")
        arc_path = create_archive(
            version=args.version,
            target=args.target,
            binary_path=pathlib.Path(args.binary).resolve(),
            out_dir=pathlib.Path(args.out_dir).resolve(),
        )

    verify_archive(
        arc_path=arc_path,
        version=args.version,
        target=args.target,
        require_static_crt=args.require_static_crt,
        skip_exec=args.skip_exec,
    )

    digest = sha256_file(arc_path)
    size = arc_path.stat().st_size
    print(f"OK: {arc_path} ({size} bytes, sha256={digest})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
