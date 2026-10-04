// v2.22.33 - Preserve exportable evidence and disclose attachments already missing on disk.

const EXPORTER_PATH: &str = "com/ofairyo/gridtimer/diagnostics/DiagnosticsExporter.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    if path != EXPORTER_PATH {
        return Ok(base.to_owned());
    }
    let mut source = base.to_owned();
    replace(&mut source,
        "            val file = File(\n                directory,\n                \"grid_timer_data_",
        "            var file = File(\n                directory,\n                \"grid_timer_data_")?;
    replace(&mut source,
        "                val workspaceAttachments = readFrozenWorkspaceAttachmentReferences(databaseBackup)",
        "                val workspaceAttachments = readFrozenWorkspaceAttachmentReferences(databaseBackup)\n                val missingMediaFiles = linkedSetOf<String>()")?;
    replace(&mut source,
        "                        inventory = workspaceAttachments\n",
        "                        inventory = workspaceAttachments,\n                        missingFiles = missingMediaFiles\n")?;
    replace(&mut source,
        "                    appStateFile(appContext).takeIf(File::exists)?.let { fileOnDisk ->",
        "                    writeZipTextEntry(zipStream, \"export_completeness.txt\", buildExportCompleteness(missingMediaFiles))\n                    appStateFile(appContext).takeIf(File::exists)?.let { fileOnDisk ->")?;
    replace(&mut source,
        "                    expectedActiveAppDataRaw = frozenCurrent.raw\n                )\n                check(!file.exists())",
        "                    expectedActiveAppDataRaw = frozenCurrent.raw,\n                    expectedCompleteness = buildExportCompleteness(missingMediaFiles)\n                )\n                if (missingMediaFiles.isNotEmpty()) {\n                    file = File(directory, file.name.replace(\"grid_timer_data_\", \"grid_timer_data_missing_${missingMediaFiles.size}_\"))\n                }\n                check(!file.exists())")?;
    replace(&mut source,
        "        inventory: FrozenAttachmentInventory\n    ): String {",
        "        inventory: FrozenAttachmentInventory,\n        missingFiles: MutableSet<String>\n    ): String {")?;
    replace(
        &mut source,
        r####"                        val validFile = runCatching {
                            File(mediaDirectory, reference.fileName).canonicalFile
                        }.getOrNull()?.takeIf { file ->
                            file.isFile && file.parentFile == mediaDirectory
                        } ?: error(
                            "Referenced note media is missing or escaped its workspace: " +
                                "${reference.workspaceKey}/${reference.fileName}"
                        )"####,
        r####"                        val validFile = File(mediaDirectory, reference.fileName).canonicalFile
                        check(validFile.parentFile == mediaDirectory &&
                            reference.fileName.isNotBlank() &&
                            '/' !in reference.fileName && '\\' !in reference.fileName) {
                            "Referenced note media escaped its workspace: ${reference.fileName}"
                        }
                        if (!validFile.exists()) {
                            missingFiles += "${reference.workspaceKey}/${reference.fileName}"
                            appendMediaManifestRow(reference, "", "missing", reference.expectedSha256)
                            return@referenceLoop
                        }
                        check(validFile.isFile) { "Referenced note media is not a regular file: ${reference.fileName}" }"####,
    )?;
    replace(&mut source,
        "                        val bytes = validFile.readBytes()\n                        val digest = sha256(bytes)",
        "                        val digest = sha256(validFile)")?;
    replace(
        &mut source,
        "                            size = bytes.size.toLong(),",
        "                            size = validFile.length(),",
    )?;
    replace(
        &mut source,
        r####"                            writeZipBytesEntry(
                                zipStream = zipStream,
                                entryName = newBlob.entryName,
                                bytes = bytes
                            )"####,
        r####"                            val written = writeZipFileEvidenceEntry(zipStream, newBlob.entryName, validFile)
                            check(written.size == newBlob.size && written.sha256 == digest) {
                                "Referenced note media changed while exporting: ${reference.fileName}"
                            }"####,
    )?;
    replace(&mut source,
        "    private fun verifyExportZip(file: File, expectedActiveAppDataRaw: String) {",
        "    private fun verifyExportZip(file: File, expectedActiveAppDataRaw: String, expectedCompleteness: String) {")?;
    replace(&mut source,
        "                \"note_media/manifest.tsv\"\n            )",
        "                \"note_media/manifest.tsv\",\n                \"export_completeness.txt\"\n            )")?;
    replace(
        &mut source,
        r####"                val expectedText = expectedActiveAppDataRaw.takeIf {
                    entry.name == "app_data.json" || entry.name == STATE_FILE_NAME
                }"####,
        r####"                val expectedText = when (entry.name) {
                    "app_data.json", STATE_FILE_NAME -> expectedActiveAppDataRaw
                    "export_completeness.txt" -> expectedCompleteness
                    else -> null
                }"####,
    )?;
    replace(
        &mut source,
        "    private fun writeWorkspaceMedia(",
        &format!("{COMPLETENESS}    private fun writeWorkspaceMedia("),
    )?;
    Ok(source)
}

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("backup availability anchor changed: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

const COMPLETENESS: &str = r####"    private fun buildExportCompleteness(missingFiles: Set<String>): String = buildString {
        appendLine("format=1")
        appendLine("complete=${missingFiles.isEmpty()}")
        appendLine("missing_file_count=${missingFiles.size}")
        appendLine("database_records=preserved")
        appendLine("attachment_reference_details=note_media/manifest.tsv")
        if (missingFiles.isNotEmpty()) {
            appendLine("部分引用附件在本机已不存在。数据库原文和现存附件已保留，缺失附件未被删除或伪造。此数据包不是完整附件备份。")
            appendLine("[missing_files]")
            missingFiles.sorted().forEach { appendLine(tsvCell(it)) }
        }
    }

"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered() -> String {
        let base = super::super::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == EXPORTER_PATH)
            .unwrap()
            .contents;
        let source = super::super::diagnostics_export_backend::render(EXPORTER_PATH, base).unwrap();
        render(EXPORTER_PATH, &source).unwrap()
    }

    #[test]
    fn missing_attachment_is_disclosed_without_weakening_path_or_hash_guards() {
        let source = rendered();
        assert!(source.contains("validFile.parentFile == mediaDirectory"));
        assert!(source.contains(
            "appendMediaManifestRow(reference, \"\", \"missing\", reference.expectedSha256)"
        ));
        assert!(source.contains("Referenced note media hash mismatch"));
        assert!(source.contains("Referenced note media changed while exporting"));
        assert!(source.contains("grid_timer_data_missing_${missingMediaFiles.size}_"));
        assert!(source.contains("complete=${missingFiles.isEmpty()}"));
    }

    #[test]
    fn files_and_completeness_are_streamed_and_verified_before_publication() {
        let source = rendered();
        assert!(!source.contains("val bytes = validFile.readBytes()"));
        assert!(
            source.contains("expectedCompleteness = buildExportCompleteness(missingMediaFiles)")
        );
        assert!(source.contains("\"export_completeness.txt\" -> expectedCompleteness"));
        assert!(source.contains("createVerifiedBackup(databaseBackup)"));
        assert!(source.contains("AtomicFileStore.replaceWithSyncedFile(file, temporaryZip)"));
    }
}
