"""Replay the actual client rustc argv against isolated diagnostic mutation copies.

No Cargo is invoked. The production sources/target are read-only inputs. Only
source path, --out-dir and opt-level are changed. Performance uses the separately
measured actual release executable, never these opt-level=0 diagnostic binaries.
"""
from pathlib import Path
import argparse
import ctypes
import hashlib
import json
import os
import re
import subprocess
import time

REPO = Path(__file__).resolve().parents[1]
VERIFICATION = REPO / "release_artifacts/verification"
PRODUCTION = REPO / "native/gridtimer_native"
TARGET = Path("C:/gt/gridtimer-build/windows-release")
BINDING_ENV = (
    "GRIDTIMER_EXPECTED_SYNC_SERVER_SHA256", "GRIDTIMER_EXPECTED_SYNC_SERVER_SIZE",
    "GRIDTIMER_EXPECTED_SYNC_SERVER_BINDING_V1", "GRIDTIMER_EXPECTED_SYNC_LAUNCHER_SHA256",
    "GRIDTIMER_EXPECTED_SYNC_LAUNCHER_SIZE", "GRIDTIMER_EXPECTED_SYNC_LAUNCHER_BINDING_V1",
    "GRIDTIMER_GIT_COMMIT", "GRIDTIMER_SOURCE_SNAPSHOT_SHA256",
)
LIBRARY_SUFFIXES = {".rlib", ".rmeta", ".lib", ".a", ".dll", ".res"}


def sha(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def save(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8", newline="\n")


def windows_argv(command_line):
    if os.name != "nt":
        raise RuntimeError("CommandLineToArgvW is required for an unparsed Windows command line")
    shell = ctypes.WinDLL("shell32", use_last_error=True)
    shell.CommandLineToArgvW.argtypes = [ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_int)]
    shell.CommandLineToArgvW.restype = ctypes.POINTER(ctypes.c_wchar_p)
    count = ctypes.c_int()
    result = shell.CommandLineToArgvW(command_line, ctypes.byref(count))
    if not result:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        return [result[index] for index in range(count.value)]
    finally:
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.LocalFree.argtypes = [ctypes.c_void_p]
        kernel.LocalFree.restype = ctypes.c_void_p
        kernel.LocalFree(result)


def option_slots(args, option):
    matches = []
    for index, argument in enumerate(args):
        if argument == option:
            if index + 1 == len(args):
                raise ValueError("Missing option value: " + option)
            matches.append((index + 1, args[index + 1], None))
        elif argument.startswith(option + "="):
            matches.append((index, argument[len(option) + 1:], option + "="))
    return matches


def unique_option(args, option):
    slots = option_slots(args, option)
    if len(slots) != 1:
        raise ValueError("Expected exactly one " + option)
    return slots[0]


def codegen_slots(args):
    matches = []
    for index, argument in enumerate(args):
        if argument == "-C":
            if index + 1 == len(args):
                raise ValueError("Missing -C value")
            matches.append((index + 1, args[index + 1], None))
        elif argument.startswith("-C") and len(argument) > 2:
            matches.append((index, argument[2:], "-C"))
    return matches


def rewrite_arguments(arguments, working, isolated_source, isolated_output):
    args = list(arguments)
    if not all(isinstance(item, str) for item in args) or any(item.startswith("@") for item in args):
        raise ValueError("An explicit argv without response files is required")
    if "--test" not in args or unique_option(args, "--crate-name")[1] != "timer_windows_client":
        raise ValueError("Capture the actual client test compiler, not another binary/library")
    if unique_option(args, "--target")[1] != "x86_64-pc-windows-msvc":
        raise ValueError("Only the actual Windows MSVC target is accepted")
    if any(item.startswith("-o") or item.startswith("--output") for item in args):
        raise ValueError("Explicit output options could escape the isolated output directory")
    emits = unique_option(args, "--emit")[1].split(",")
    if set(emits) != {"dep-info", "link"}:
        raise ValueError("Only plain dep-info/link outputs are accepted")
    source = (working / "src/bin/timer_windows_client.rs").resolve()
    source_slots = [index for index, argument in enumerate(args)
                    if argument.endswith(".rs") and (working / argument).resolve() == source]
    if len(source_slots) != 1:
        raise ValueError("The captured source is not the exact production client")
    out_slot = unique_option(args, "--out-dir")
    codegen = codegen_slots(args)
    optimization = [slot for slot in codegen if slot[1].startswith("opt-level=")]
    if len(optimization) != 1 or optimization[0][1] != "opt-level=3":
        raise ValueError("Capture the actual optimized release compiler argv")
    if any(slot[1].startswith("incremental=") for slot in codegen):
        raise ValueError("Incremental outputs must be disabled before capturing the actual compiler")
    extra = [slot[1].split("=", 1)[1] for slot in codegen if slot[1].startswith("extra-filename=")]
    if len(extra) != 1 or not re.fullmatch(r"[-A-Za-z0-9_]+", extra[0]):
        raise ValueError("The actual client artifact suffix is missing or unsafe")
    source_index = source_slots[0]
    args[source_index] = str(isolated_source)
    args[out_slot[0]] = (out_slot[2] or "") + str(isolated_output)
    opt_slot = optimization[0]
    args[opt_slot[0]] = (opt_slot[2] or "") + "opt-level=0"
    changed = [index for index, (before, after) in enumerate(zip(arguments, args)) if before != after]
    if set(changed) != {source_index, out_slot[0], opt_slot[0]}:
        raise ValueError("Replay may only alter source, output and diagnostic opt-level")
    return args, "timer_windows_client" + extra[0] + ".exe", {
        "changedArgumentIndices": changed,
        "sourceIndex": source_index, "outputIndex": out_slot[0], "optLevelIndex": opt_slot[0],
        "originalSource": str(source), "originalOutputDirectory": out_slot[1],
        "originalOptLevel": 3, "diagnosticOptLevel": 0,
    }


def normalize_capture(record):
    argv = record.get("argv")
    if argv is None and record.get("commandLine"):
        argv = windows_argv(record["commandLine"])
    if not argv or not all(isinstance(item, str) for item in argv):
        raise ValueError("Capture JSON needs full argv including the actual rustc executable")
    executable = Path(argv[0]).resolve()
    if executable.name.lower() != "rustc.exe" or not executable.is_file():
        raise ValueError("Use the actual captured rustc.exe path")
    working = Path(record["workingDirectory"]).resolve()
    if working != PRODUCTION.resolve():
        raise ValueError("Capture must originate in the real native package working directory")
    if "environment" not in record or "LIB" not in record["environment"]:
        raise ValueError("Record the compiler environment including LIB (null if unset)")
    return executable, argv[1:], working, record["environment"]


def library_inputs(executable, args, working, environment):
    files = {executable}
    directories = set()
    sysroots = option_slots(args, "--sysroot")
    if len(sysroots) > 1:
        raise ValueError("The captured compiler has ambiguous sysroot options")
    sysroot = (working / sysroots[0][1]).resolve() if sysroots else executable.parent.parent
    standard_library = sysroot / "lib/rustlib" / unique_option(args, "--target")[1] / "lib"
    if not standard_library.is_dir():
        raise ValueError("Missing actual compiler standard library directory: " + str(standard_library))
    directories.add(standard_library)
    # rustc.exe loads its driver and host standard-library DLLs from the actual
    # toolchain bin directory; lock those as well as the target standard library.
    files.update(path.resolve() for path in executable.parent.iterdir()
                 if path.is_file() and path.suffix.lower() == ".dll")
    for _, value, _ in option_slots(args, "--extern"):
        if "=" not in value:
            raise ValueError("All actual extern crates require explicit artifact paths")
        artifact = (working / value.split("=", 1)[1]).resolve()
        if not artifact.is_file():
            raise ValueError("Missing captured extern artifact: " + str(artifact))
        files.add(artifact)
    for index, argument in enumerate(args):
        value = args[index + 1] if argument == "-L" and index + 1 < len(args) else None
        if argument.startswith("-L") and len(argument) > 2:
            value = argument[2:]
        if value:
            directory = (working / value.split("=", 1)[-1]).resolve()
            if not directory.is_dir():
                raise ValueError("Missing actual native/dependency directory: " + str(directory))
            directories.add(directory)
    for _, value, _ in codegen_slots(args):
        if value.startswith("linker="):
            linker = (working / value.split("=", 1)[1]).resolve()
            if not linker.is_file():
                raise ValueError("Linker must be an explicit existing path")
            files.add(linker)
        elif value.startswith("link-arg="):
            link_value = value.split("=", 1)[1]
            if link_value.lower().endswith(".res"):
                resource = (working / link_value).resolve()
                if not resource.is_file():
                    raise ValueError("Missing actual build.rs resource")
                files.add(resource)
    for directory in str(environment.get("LIB") or "").split(";"):
        if directory:
            directory = Path(directory).resolve()
            if not directory.is_dir():
                raise ValueError("Captured LIB directory no longer exists: " + str(directory))
            directories.add(directory)
    # Lock direct artifacts and the transitive/native resolver inputs. The
    # compiler can only read these directories; every output lives elsewhere.
    for directory in directories:
        files.update(path.resolve() for path in directory.iterdir()
                     if path.is_file() and path.suffix.lower() in LIBRARY_SUFFIXES)
    return {str(path): sha(path) for path in sorted(files, key=lambda path: str(path).lower())}


def assert_hashes(files):
    for path, expected in files.items():
        if not Path(path).is_file() or sha(path) != expected:
            raise RuntimeError("Locked compiler/source input changed: " + path)


def invoke(argv, cwd, environment, prefix, timeout):
    began = time.time()
    stdout_path = Path(str(prefix) + ".stdout.log")
    stderr_path = Path(str(prefix) + ".stderr.log")
    timed_out = False
    with stdout_path.open("w", encoding="utf-8") as stdout, stderr_path.open("w", encoding="utf-8") as stderr:
        process = subprocess.Popen(argv, cwd=cwd, env=environment, stdout=stdout, stderr=stderr,
                                   creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        progress = time.monotonic() + 15
        deadline = time.monotonic() + timeout
        while process.poll() is None:
            if time.monotonic() >= progress:
                print(prefix.name + " elapsed " + str(int(time.time() - began)) + "s", flush=True)
                progress += 15
            if time.monotonic() >= deadline:
                # PID comes only from our still-live Popen handle; never target
                # an application name, port, or unrelated compiler process.
                subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                               capture_output=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
                process.wait(timeout=30)
                timed_out = True
                break
            time.sleep(0.25)
        exit_code = process.wait()
    result = {"argv": argv, "workingDirectory": str(cwd), "pid": process.pid,
              "beganUnixSeconds": began, "elapsedSeconds": time.time() - began,
              "exitCode": exit_code, "timedOut": timed_out,
              "stdoutSha256": sha(stdout_path), "stderrSha256": sha(stderr_path)}
    save(str(prefix) + ".process.json", result)
    return result, stdout_path.read_text(encoding="utf-8"), stderr_path.read_text(encoding="utf-8")


def static_check():
    working = PRODUCTION.resolve()
    source = VERIFICATION / "static_fixture/native/gridtimer_native/src/bin/timer_windows_client.rs"
    output = VERIFICATION / "static_fixture/compiler_output"
    args = ["--crate-name", "timer_windows_client", "--edition=2021", "src/bin/timer_windows_client.rs",
            "--test", "--emit=dep-info,link", "-C", "opt-level=3", "-C", "extra-filename=-actualhash",
            "--out-dir", str(TARGET / "x86_64-pc-windows-msvc/release/deps"),
            "--target", "x86_64-pc-windows-msvc", "--extern", "gridtimer_native=C:/actual/core.rlib",
            "-L", "dependency=C:/actual/deps", "-C", "link-arg=C:/actual/icon.res"]
    changed, name, record = rewrite_arguments(args, working, source, output)
    assert len(record["changedArgumentIndices"]) == 3 and name == "timer_windows_client-actualhash.exe"
    assert "gridtimer_native=C:/actual/core.rlib" in changed and "link-arg=C:/actual/icon.res" in changed
    blocked = 0
    for bad in [args + ["-o", "C:/production/overwrite.exe"], args + ["-C", "incremental=C:/production/cache"],
                [item.replace("dep-info,link", "dep-info,link=C:/production/overwrite.exe") for item in args],
                [item.replace("opt-level=3", "opt-level=2") for item in args],
                [item.replace("src/bin/timer_windows_client.rs", "src/bin/timer_sync_server.rs") for item in args],
                args + ["@C:/unknown/response_file"]]:
        try:
            rewrite_arguments(bad, working, source, output)
        except ValueError:
            blocked += 1
        else:
            raise AssertionError("Unsafe compiler argv accepted")
    assert blocked == 6
    print("DIRECT_RUSTC_REPLAY_STATIC_RULES_OK allowed_changes=3 blocked_unsafe_cases=6; no process launched")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--static-check", action="store_true")
    parser.add_argument("--compiler-record")
    parser.add_argument("--mutation-manifest")
    parser.add_argument("--output")
    parser.add_argument("--variants", nargs="+", default=["wording_only", "remove_encrypted_parent_boundary"])
    parser.add_argument("--compile-timeout", type=int, default=1800)
    parser.add_argument("--test-timeout", type=int, default=180)
    args = parser.parse_args()
    if args.static_check:
        static_check()
        return
    if not all([args.compiler_record, args.mutation_manifest, args.output]):
        parser.error("compiler record, immutable mutation manifest and fresh verification output are required")
    output = Path(args.output).resolve()
    manifest_path = Path(args.mutation_manifest).resolve()
    if not output.is_relative_to(VERIFICATION.resolve()) or not manifest_path.is_relative_to(VERIFICATION.resolve()):
        raise ValueError("All copies/output must remain under this project verification directory")
    if output.exists():
        raise ValueError("Keep earlier evidence; choose a new output directory")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    if manifest.get("formatVersion") != 2 or not manifest.get("unmodified"):
        raise ValueError("Use verified format-2 immutable mutation copies")
    variants = {item["name"]: item for item in manifest["variants"]}
    if any(name not in variants for name in args.variants):
        raise ValueError("Unknown requested mutation")
    selected = [manifest["unmodified"], *[variants[name] for name in args.variants]]
    if any(not re.fullmatch(r"[A-Za-z0-9_-]+", item["name"]) for item in selected):
        raise ValueError("Isolated variant names cannot contain paths")
    capture_path = Path(args.compiler_record).resolve()
    capture = json.loads(capture_path.read_text(encoding="utf-8-sig"))
    executable, original_args, working, captured_environment = normalize_capture(capture)
    # Reject unsafe argv before creating output or launching any process.
    rewrite_arguments(original_args, working, Path(selected[0]["packageDirectory"]) / "src/bin/timer_windows_client.rs", output / "unmodified")
    original_out = Path(unique_option(original_args, "--out-dir")[1]).resolve()
    if not original_out.is_relative_to((TARGET / "x86_64-pc-windows-msvc/release").resolve()):
        raise ValueError("Actual compiler record must come from the single shared release target")
    sources = {str(REPO / relative): expected.lower() for relative, expected in manifest["sourceFiles"].items()}
    assert_hashes(sources)
    library_hashes = library_inputs(executable, original_args, working, captured_environment)
    output.mkdir(parents=True)
    save(output / "locked_inputs.json", {"compilerRecordSha256": sha(capture_path),
         "mutationManifestSha256": sha(manifest_path), "productionSourceFiles": sources,
         "externalCompilerAndLibraryInputs": library_hashes,
         "profile": "unoptimized-isolated-client-mutation; actual release extern/native artifacts",
         "notPerformanceEvidence": True})
    environment = os.environ.copy()
    for key in BINDING_ENV:
        environment.pop(key, None)
    for key, value in captured_environment.items():
        if key in BINDING_ENV or key in {"LIB", "PATH", "RUSTUP_TOOLCHAIN", "CARGO_HOME"}:
            if value is None:
                environment.pop(key, None)
            else:
                environment[key] = str(value)
    environment.pop("DESKTOP_UI_CACHE_PERFORMANCE_OUTPUT", None)
    results = []
    for variant in selected:
        assert_hashes(sources)
        assert_hashes(library_hashes)
        package = Path(variant["packageDirectory"]).resolve()
        copy_root = package.parents[1]
        if not package.is_relative_to(manifest_path.parent) or package == working:
            raise ValueError("Variant must be an isolated immutable input copy")
        if any(not (copy_root / relative).resolve().is_relative_to(copy_root) for relative in variant["inputFiles"]):
            raise ValueError("Copy input paths cannot escape their isolated root")
        copy_hashes = {str(copy_root / relative): expected.lower() for relative, expected in variant["inputFiles"].items()}
        assert_hashes(copy_hashes)
        variant_output = output / variant["name"]
        variant_output.mkdir()
        replay, filename, changes = rewrite_arguments(original_args, working, package / "src/bin/timer_windows_client.rs", variant_output)
        variant_environment = dict(environment)
        variant_environment["CARGO_MANIFEST_DIR"] = str(package)
        variant_environment["LOCALAPPDATA"] = str(variant_output / "startup_synthetic_ui_mutation")
        Path(variant_environment["LOCALAPPDATA"]).mkdir()
        save(variant_output / "compiler_replay.json", {"executable": str(executable), "argv": replay,
            "changes": changes, "profile": "unoptimized isolated mutation", "notPerformanceEvidence": True,
            "compileBindingEnvironmentSha256": hashlib.sha256(json.dumps({key: variant_environment.get(key) for key in BINDING_ENV}, sort_keys=True).encode()).hexdigest()})
        build, _, diagnostics = invoke([str(executable), *replay], working, variant_environment,
                                       variant_output / "compile", args.compile_timeout)
        artifact = variant_output / filename
        if build["exitCode"] != 0 or build["timedOut"] or not artifact.is_file():
            raise RuntimeError("Compilation must succeed before a mutation can be evaluated: " + variant["name"])
        artifact_sha = sha(artifact)
        tests = []
        for index, test in enumerate(variant["exactTests"]):
            run, stdout, stderr = invoke([str(artifact), test, "--exact", "--nocapture", "--test-threads=1"],
                                         package, variant_environment, variant_output / ("test_" + str(index + 1)), args.test_timeout)
            if run["timedOut"] or "running 1 test" not in stdout:
                raise RuntimeError("Mutation never reached the exact business test: " + test)
            if variant["expectedTestOutcome"] == "pass":
                valid = run["exitCode"] == 0 and "1 passed; 0 failed" in stdout
            else:
                valid = run["exitCode"] == 101 and "0 passed; 1 failed" in stdout and "panicked at" in stderr and "assertion" in stderr
            if not valid:
                raise RuntimeError("Mutation expectation not met by a Rust business assertion: " + variant["name"] + " / " + test)
            tests.append({"exactTest": test, "expectedOutcome": variant["expectedTestOutcome"], "verified": True,
                          "executableSha256": artifact_sha, "process": run})
        assert_hashes(copy_hashes)
        assert_hashes(sources)
        assert_hashes(library_hashes)
        if sha(artifact) != artifact_sha:
            raise RuntimeError("Diagnostic test executable changed during its run")
        result = {"variant": variant["name"], "verified": True, "executable": str(artifact),
                  "executableSha256": artifact_sha, "build": build, "changes": changes, "tests": tests,
                  "productionSourcesAndExternalArtifactsUnchanged": True, "notPerformanceEvidence": True}
        results.append(result)
        save(variant_output / "result.json", result)
        save(output / "mutation_results.json", {"profile": "unoptimized isolated client mutations",
             "cargoInvoked": False, "libraryRebuilt": False, "productionTargetWritten": False,
             "notPerformanceEvidence": True, "results": results})
    print(output / "mutation_results.json", flush=True)


if __name__ == "__main__":
    main()
