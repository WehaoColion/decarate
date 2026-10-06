#!/usr/bin/env python3
"""Compile and execute the actual emitted reader policy; no Compose mocks.
Run from any directory: python tools/verify_structured_reader.py
Requires Kotlin/JVM and Java on PATH. --mutations also proves two regression guards.
"""
from __future__ import annotations
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

TESTS = r'''
private var cases = 0
private fun test(name: String, action: () -> Unit) {
    try { action(); cases++; println("PASS: $name") }
    catch (error: Throwable) { throw AssertionError("FAIL: $name", error) }
}
private fun b(id: String, kind: String = "paragraph", parent: String = "", text: String = id,
    collapsed: Boolean = false, checked: Boolean = false) = ReaderBlock(id, kind, text, parent, collapsed, checked)
fun main() {
    test("empty page") { check(readerPlan(emptyList()).blocks.isEmpty()) }
    test("root order") { check(readerVisible(readerPlan(listOf(b("c"), b("a"), b("b"))), emptySet()) == listOf(0,1,2)) }
    val nested = readerPlan(listOf(b("a","toggle",collapsed=true), b("b","toggle","a",collapsed=true), b("c",parent="b"), b("d")))
    test("initial folds use stored flags") { check(readerInitialFolds(nested) == setOf(0,1)) }
    test("collapsed ancestor hides descendants") { check(readerVisible(nested,setOf(0)) == listOf(0,3)) }
    test("inner fold keeps outer header") { check(readerVisible(nested,setOf(1)) == listOf(0,1,3)) }
    test("nested folds independent") { check(readerToggle(nested,0,setOf(0,1)) == setOf(1)) }
    test("collapse toggle") { check(readerToggle(nested,1,emptySet()) == setOf(1)) }
    test("cannot collapse paragraph") { check(readerToggle(nested,2,setOf(0)) == setOf(0)) }
    test("invalid fold target is no-op") { check(readerToggle(nested,-1,setOf(0)) == setOf(0)) }
    test("reveal opens every ancestor") { check(readerReveal(nested,2,setOf(0,1)) == emptySet<Int>()) }
    test("reveal leaves unrelated fold") { check(readerReveal(nested,1,setOf(0,1,9)) == setOf(1,9)) }
    test("invalid reveal no-op") { check(readerReveal(nested,99,setOf(0)) == setOf(0)) }
    test("reveal then locate exact block") { val v=readerVisible(nested,readerReveal(nested,2,setOf(0,1))); check(readerLazyPosition(v,2)==3) }
    test("metadata offset") { check(readerLazyPosition(listOf(8,4),8)==1); check(readerLazyPosition(listOf(8,4),4)==2) }
    test("missing target not first row") { check(readerLazyPosition(listOf(8,4),0)==null) }
    test("duplicate ids preserve all rows") { val p=readerPlan(listOf(b("x"),b("x"))); check(readerVisible(p,emptySet())==listOf(0,1)) }
    test("ambiguous parent never hides child") { val p=readerPlan(listOf(b("x","toggle"),b("x","toggle"),b("c",parent="x"))); check(2 in p.malformed); check(2 in readerVisible(p,setOf(0,1))) }
    test("missing parent flagged and visible") { val p=readerPlan(listOf(b("c",parent="missing"))); check(p.malformed==setOf(0)); check(readerVisible(p,setOf(0))==listOf(0)) }
    test("self cycle is visible") { val p=readerPlan(listOf(b("a","toggle","a"))); check(p.malformed==setOf(0)); check(readerVisible(p,setOf(0))==listOf(0)) }
    test("two-node cycle and descendants visible") { val p=readerPlan(listOf(b("a","toggle","b"),b("b","toggle","a"),b("c",parent="a"))); check(p.malformed==setOf(0,1,2)); check(readerVisible(p,setOf(0,1))==listOf(0,1,2)) }
    test("depth guard degrades visibly") { val p=readerPlan((0..70).map { b("$it","toggle",if(it==0) "" else "${it-1}") }); check(70 in p.malformed); check(70 in readerVisible(p,setOf(0))) }
    test("valid depth not flattened early") { val p=readerPlan((0..64).map { b("$it","toggle",if(it==0) "" else "${it-1}") }); check(p.ancestors[64].size==64); check(64 !in p.malformed) }
    test("forward parent reference supported") { val p=readerPlan(listOf(b("c",parent="p"),b("p","toggle"))); check(p.ancestors[0]==listOf(1)); check(readerVisible(p,setOf(1))==listOf(1)) }
    test("ordinary parent not a folding control") { val p=readerPlan(listOf(b("p"),b("c",parent="p"))); check(readerVisible(p,setOf(0))==listOf(0,1)) }
    test("numbered roots increment") { val p=readerPlan(listOf(b("a","numbered_list"),b("b","numbered_list"))); check(p.ordinals==listOf(1,2)) }
    test("numbered run resets at paragraph") { val p=readerPlan(listOf(b("a","numbered_list"),b("b"),b("c","numbered_list"))); check(p.ordinals==listOf(1,null,1)) }
    test("nested numbering does not break root run") { val p=readerPlan(listOf(b("a","numbered_list"),b("c","numbered_list","a"),b("d","numbered_list","a"),b("b","numbered_list"))); check(p.ordinals==listOf(1,1,2,2)) }
    test("different parent numbering independent") { val p=readerPlan(listOf(b("a","toggle"),b("x","numbered_list","a"),b("b","toggle"),b("y","numbered_list","b"))); check(p.ordinals==listOf(null,1,null,1)) }
    test("heading directory ordered not title-deduplicated") { val p=readerPlan(listOf(b("x","heading2",text="同名"),b("y","heading1",text="同名"),b("z"))); check(p.headings==listOf(0,1)) }
    test("code hash not a heading") { check(readerPlan(listOf(b("x","code",text="# 标题"))).headings.isEmpty()) }
    test("tasks count only real todo blocks") { val p=readerPlan(listOf(b("a","todo",checked=true),b("b","todo"),b("c",text="[ ] abc"))); check(p.tasks==listOf(0,1)); check(p.pending==listOf(1)) }
    test("search includes hidden blocks") { check(readerMatches(nested,"c")==listOf(2)) }
    test("case and whitespace query") { val p=readerPlan(listOf(b("a",text="Alpha 中"))); check(readerMatches(p,"  alpha ")==listOf(0)) }
    test("literal regex metacharacters") { val p=readerPlan(listOf(b("a",text=".*"),b("b",text="其他"))); check(readerMatches(p,".*")==listOf(0)) }
    test("Chinese and emoji search") { val p=readerPlan(listOf(b("a",text="人物🙂设定"))); check(readerMatches(p,"🙂设定")==listOf(0)) }
    test("blank query no results") { check(readerMatches(nested," \n ").isEmpty()) }
    test("source text never rewritten") { val source=listOf(b("a","toggle",text="  原文\n "),b("b",parent="a")); val copy=source.toList(); val p=readerPlan(source); readerReveal(p,1,setOf(0)); readerMatches(p,"原文"); check(source==copy); check(p.blocks==copy) }
    test("workspaces never share fold state") { check(readerInitialFolds(readerPlan(listOf(b("a","toggle"))))==emptySet<Int>()) }
    test("cooperative plan cancellation") { var n=0; val cancelled=runCatching { readerPlan(List(100){b("$it")}) { n++; if(n==3) throw java.util.concurrent.CancellationException() } }; check(cancelled.exceptionOrNull() is java.util.concurrent.CancellationException); check(n==3) }
    test("cooperative search cancellation") { var n=0; val cancelled=runCatching { readerMatches(nested,"c") { n++; if(n==2) throw java.util.concurrent.CancellationException() } }; check(cancelled.exceptionOrNull() is java.util.concurrent.CancellationException) }
    test("large page is complete") { val p=readerPlan(List(5000){b("$it","numbered_list")}); check(readerVisible(p,emptySet()).size==5000); check(p.ordinals.last()==5000) }
    println("$cases reader policy cases passed")
}
'''

def policy_from_source(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    start, end = "// BEGIN STRUCTURED READER POLICY", "// END STRUCTURED READER POLICY"
    if text.count(start) != 1 or text.count(end) != 1:
        raise RuntimeError("Policy markers missing or duplicated")
    return text.split(start, 1)[1].split(end, 1)[0]

def run_case(compiler: str, java: str, root: Path, policy: str, name: str) -> subprocess.CompletedProcess[str]:
    source = root / (name + ".kt")
    jar = root / (name + ".jar")
    source.write_text(policy + "\n" + TESTS, encoding="utf-8")
    command = [compiler, str(source), "-include-runtime", "-d", str(jar)]
    # Windows distributions expose kotlinc.bat; cmd is needed for that launcher.
    if os.name == "nt" and compiler.lower().endswith((".bat", ".cmd")):
        command = [os.environ.get("COMSPEC", "cmd.exe"), "/c", *command]
    subprocess.run(command, check=True, timeout=180)
    return subprocess.run([java, "-jar", str(jar)], capture_output=True, text=True, encoding="utf-8", timeout=60)

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mutations", action="store_true")
    args = parser.parse_args()
    compiler, java = shutil.which("kotlinc"), shutil.which("java")
    if not compiler or not java:
        raise SystemExit("Kotlin/JVM and Java are required on PATH; no tests ran.")
    repo = Path(__file__).resolve().parents[1]
    policy = policy_from_source(repo / "native/gridtimer_native/src/sourcegen/android_structured_reader.rs")
    with tempfile.TemporaryDirectory(prefix="structured_reader_") as work:
        base = run_case(compiler, java, Path(work), policy, "baseline")
        print(base.stdout, end="")
        if base.returncode:
            raise SystemExit(base.stderr or "Reader policy checks failed")
        if args.mutations:
            mutations = [
                ("ancestor_reveal", "folded - plan.ancestors[target].toSet()", "folded", "reveal opens every ancestor"),
                ("numbering", "((runs[parent] ?: 0) + 1)", "(1)", "numbered roots increment"),
            ]
            for name, before, after, expected in mutations:
                if policy.count(before) != 1:
                    raise RuntimeError("Mutation anchor drift: " + name)
                result = run_case(compiler, java, Path(work), policy.replace(before, after, 1), name)
                if result.returncode == 0 or ("FAIL: " + expected) not in result.stderr:
                    raise RuntimeError("Mutation was not caught by the intended check: " + name + "\n" + result.stderr)
                print("CAUGHT mutation: " + name)

if __name__ == "__main__":
    main()
