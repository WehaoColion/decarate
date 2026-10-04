"""Prepare isolated knowledge read-cache business mutations; never builds product source."""
from pathlib import Path
import json
import sys

import prepare_windows_timer_mutations as preparation


def knowledge_ui_mutations():
    return [
        {
            "name": "wording_only",
            "file": "src/desktop/knowledge_experience.rs",
            "old": 'egui::Window::new("快速跳转")',
            "new": 'egui::Window::new("页面跳转")',
            "expect": "pass",
            "tests": [
                "tests::knowledge_quick_navigation_cache_tracks_query_history_notes_and_workspace",
                "tests::knowledge_quick_navigation_never_exposes_encrypted_or_deleted_page_titles",
                "tests::knowledge_database_frame_cache_preserves_order_query_and_snapshot_boundaries",
            ],
        },
        {
            "name": "remove_encrypted_parent_boundary",
            "file": "src/desktop/knowledge_experience.rs",
            "old": '                .map(|index| &navigation.pages[*index])\n                .filter(|page| !page.encrypted)\n                .map_or("知识库", |page| page.title.as_str());',
            "new": '                .map(|index| &navigation.pages[*index])\n                .map_or("知识库", |page| page.title.as_str());',
            "expect": "fail",
            "tests": [
                "tests::knowledge_quick_navigation_never_exposes_encrypted_or_deleted_page_titles",
            ],
        },
        {
            "name": "remove_notes_version_invalidation",
            "file": "src/desktop/knowledge_experience.rs",
            "old": '            cache.at_version(self.notes_cache_version());\n            if let Some(previous) = &cache.quick_choices {',
            "new": '            if let Some(previous) = &cache.quick_choices {',
            "expect": "fail",
            "tests": [
                "tests::knowledge_quick_navigation_cache_tracks_query_history_notes_and_workspace",
            ],
        },
    ]


def main():
    default_output = "release_artifacts/verification/windows_v1.1.0.3/ui_cache_mutations"
    arguments = sys.argv[1:]
    output = default_output
    if "--output" in arguments:
        output = arguments[arguments.index("--output") + 1]
    elif any(argument.startswith("--output=") for argument in arguments):
        output = next(argument.split("=", 1)[1] for argument in arguments if argument.startswith("--output="))
    else:
        sys.argv += ["--output", output]
    # Reuse the established source inventory, exact-anchor validation, immutable
    # copies, format-2 manifests and same-workspace sequential build protocol.
    preparation.mutations = knowledge_ui_mutations
    preparation.main()
    if "--check-only" in arguments:
        return
    repo = Path(__file__).resolve().parents[1]
    manifest_path = (repo / output).resolve() / "mutation_manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["purpose"] = "Knowledge navigation privacy and read-cache invalidation mutations; only compile isolated client-test copies sequentially with the shared release target"
    manifest["measurementScope"] = "Mutation results prove business boundaries. Quantified performance uses the separately hashed actual candidate test executable."
    manifest["executionDefaults"] = {
        "targetDirectory": "C:/gt/gridtimer-build/windows-release",
        "profile": "release", "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "2",
    }
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
