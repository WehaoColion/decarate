// v2.23 - Connect Android finance to the legal clue page and evidence navigation.

fn replace_once(source: String, before: &str, after: &str) -> Result<String, String> {
    if !source.contains(before) {
        return Err(format!(
            "missing legal integration anchor: {}",
            before.lines().next().unwrap_or("")
        ));
    }
    Ok(source.replacen(before, after, 1))
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    match path {
        "com/ofairyo/gridtimer/ui/GridTimerScreen.kt" => render_screen(source),
        "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt" => render_note_studio(source),
        "com/ofairyo/gridtimer/ui/SmartisanNoteUi.kt" => render_smartisan_note(source),
        _ => Ok(source.to_string()),
    }
}

fn render_screen(source: &str) -> Result<String, String> {
    let mut source = source.to_string();
    source = replace_once(
        source,
        "import com.ofairyo.gridtimer.data.NoteEntry\nimport com.ofairyo.gridtimer.data.SyncAccountSession",
        "import com.ofairyo.gridtimer.data.NoteEntry\nimport com.ofairyo.gridtimer.data.NoteEntryKind\nimport com.ofairyo.gridtimer.data.SyncAccountSession",
    )?;
    source = replace_once(
        source,
        "    var selectedDestination by rememberSaveable { mutableStateOf(HomeNavigationDestination.BOARD) }",
        "    var selectedDestination by rememberSaveable { mutableStateOf(HomeNavigationDestination.BOARD) }\n    var pendingLegalNoteId by rememberSaveable { mutableStateOf<String?>(null) }\n    var pendingLegalVersionId by rememberSaveable { mutableStateOf<String?>(null) }",
    )?;
    source = replace_once(
        source,
        "        if (selectedDestination != HomeNavigationDestination.NOTES &&\n            selectedDestination != HomeNavigationDestination.NOTEBOOK\n        ) {\n            noteSurfaceEditing = false\n        }",
        "        if (selectedDestination != HomeNavigationDestination.NOTES &&\n            selectedDestination != HomeNavigationDestination.NOTEBOOK\n        ) {\n            noteSurfaceEditing = false\n            pendingLegalNoteId = null\n            pendingLegalVersionId = null\n        }",
    )?;
    source = replace_once(
        source,
        "                                StickyNoteStudioSheet(\n                                    appData = appData,",
        "                                StickyNoteStudioSheet(\n                                    initialNoteId = pendingLegalNoteId,\n                                    initialVersionId = pendingLegalVersionId,\n                                    appData = appData,",
    )?;
    source = replace_once(
        source,
        "                                NoteStudioSheet(\n                                    appData = appData,",
        "                                NoteStudioSheet(\n                                    initialNoteId = pendingLegalNoteId,\n                                    appData = appData,",
    )?;
    source = replace_once(
        source,
        "                            FinanceSheet(\n                                profile = appData.financeProfile,",
        r####"                            FinanceSheet(
                                appData = appData,
                                syncSession = syncSession,
                                viewModel = viewModel,
                                onOpenExternalEvidence = { sourcePath ->
                                    val noteId = sourcePath.split('/').getOrNull(1)
                                    when {
                                        sourcePath.startsWith("note/") && noteId != null -> {
                                            pendingLegalNoteId = noteId
                                            val segments = sourcePath.split('/')
                                            pendingLegalVersionId = if (segments.getOrNull(2) == "versions") segments.getOrNull(3) else null
                                            selectedDestination = if (
                                                appData.notes.firstOrNull { it.id == noteId }?.kind == NoteEntryKind.STICKY
                                            ) HomeNavigationDestination.NOTES else HomeNavigationDestination.NOTEBOOK
                                        }
                                        sourcePath.startsWith("slot/") -> {
                                            sourcePath.substringAfter('/').toIntOrNull()?.let { viewModel.openSlot(it) }
                                            selectedDestination = HomeNavigationDestination.BOARD
                                        }
                                        sourcePath.startsWith("session/") || sourcePath.startsWith("archivedTask/") -> {
                                            selectedDestination = HomeNavigationDestination.BOARD
                                            historyVisible = true
                                        }
                                    }
                                },
                                profile = appData.financeProfile,"####,
    )?;
    source = replace_once(
        source,
        "private fun FinanceSheet(\n    profile: FinanceProfile,",
        "private fun FinanceSheet(\n    appData: AppData,\n    syncSession: SyncAccountSession,\n    viewModel: TimerViewModel,\n    onOpenExternalEvidence: (String) -> Unit,\n    profile: FinanceProfile,",
    )?;
    source = replace_once(
        source,
        "    val periodSelection = rememberFinancePeriodSelectionState()",
        "    var showLegalRisk by rememberSaveable(workspaceKey) { mutableStateOf(false) }\n    val periodSelection = rememberFinancePeriodSelectionState()",
    )?;
    source = replace_once(
        source,
        r####"                    Text(
                        text = "风控",
                        style = MaterialTheme.typography.headlineSmall.copy(fontWeight = FontWeight.Bold)
                    )"####,
        r####"                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            text = "风控",
                            style = MaterialTheme.typography.headlineSmall.copy(fontWeight = FontWeight.Bold)
                        )
                        TextButton(onClick = { showLegalRisk = true }) {
                            Text("法律风险线索")
                        }
                    }"####,
    )?;
    source = replace_once(
        source,
        "                FinanceReportPeriodCard(\n                    selectedPeriod = selectedPeriod,",
        "                Text(\"账目浏览周期\", style = MaterialTheme.typography.labelMedium)\n                FinanceReportPeriodCard(\n                    selectedPeriod = selectedPeriod,",
    )?;
    source = replace_once(
        source,
        "    if (asPage) {\n        Surface(\n            modifier = modifier.fillMaxSize(),\n            color = MaterialTheme.colorScheme.background\n        ) {\n            financeContent()\n        }\n    } else {\n        InAppBottomSheetOverlay(onDismiss = onDismiss, modifier = modifier) {\n            financeContent()\n        }\n    }\n}",
        r####"    if (showLegalRisk) {
        LegalRiskScreen(
            appData = appData,
            workspaceKey = workspaceKey,
            syncSession = syncSession,
            viewModel = viewModel,
            onDismiss = { showLegalRisk = false },
            onOpenEvidence = { sourcePath ->
                when {
                    sourcePath.startsWith("finance/day/") -> {
                        val dateKey = sourcePath.split('/').getOrNull(2)
                        if (dateKey != null && runCatching { LocalDate.parse(dateKey) }.isSuccess) {
                            periodSelection.selectedPeriod = FinanceReportPeriod.DAY
                            periodSelection.selectedDayKey = dateKey
                            periodSelection.dayDraft = dateKey
                            showLegalRisk = false
                            scope.launch { financeListState.animateScrollToItem(3) }
                        }
                    }
                    sourcePath.startsWith("finance/month/") -> {
                        val monthKey = sourcePath.split('/').getOrNull(2)
                        if (monthKey != null && runCatching { YearMonth.parse(monthKey) }.isSuccess) {
                            periodSelection.selectedPeriod = FinanceReportPeriod.MONTH
                            periodSelection.selectedMonthKey = monthKey
                            periodSelection.monthDraft = monthKey
                            showLegalRisk = false
                            scope.launch { financeListState.animateScrollToItem(3) }
                        }
                    }
                    else -> {
                        showLegalRisk = false
                        onOpenExternalEvidence(sourcePath)
                    }
                }
            },
            modifier = modifier
        )
    } else if (asPage) {
        Surface(
            modifier = modifier.fillMaxSize(),
            color = MaterialTheme.colorScheme.background
        ) {
            financeContent()
        }
    } else {
        InAppBottomSheetOverlay(onDismiss = onDismiss, modifier = modifier) {
            financeContent()
        }
    }
}"####,
    )?;
    Ok(source)
}

fn render_note_studio(source: &str) -> Result<String, String> {
    let mut source = source.to_string();
    for function in ["NoteStudioSheet", "StickyNoteStudioSheet"] {
        source = replace_once(
            source,
            &format!("internal fun {function}(\n    appData: AppData,"),
            &format!("internal fun {function}(\n    initialNoteId: String? = null,\n    appData: AppData,"),
        )?;
    }
    source = replace_once(
        source,
        "internal fun StickyNoteStudioSheet(\n    initialNoteId: String? = null,\n    appData: AppData,",
        "internal fun StickyNoteStudioSheet(\n    initialNoteId: String? = null,\n    initialVersionId: String? = null,\n    appData: AppData,",
    )?;
    let anchor = "    var unlockBusy by remember { mutableStateOf(false) }";
    let replacement = "    var unlockBusy by remember { mutableStateOf(false) }\n    LaunchedEffect(initialNoteId) {\n        val target = appData.notes.firstOrNull { it.id == initialNoteId && !it.isDeleted() }\n        if (target != null) {\n            if (target.isEncryptionLocked()) {\n                unlockTarget = target\n                unlockError = \"\"\n            } else {\n                selectedNoteId = target.id\n            }\n        }\n    }";
    if source.matches(anchor).count() != 2 {
        return Err("expected two note studio unlock anchors".into());
    }
    source = source.replace(anchor, replacement);
    source = replace_once(
        source,
        "                SmartisanNoteEditorContent(\n                    note = selectedNote,\n                    folders = appData.noteFolders,\n                    highlightQuery = searchLocateQuery,",
        "                SmartisanNoteEditorContent(\n                    note = selectedNote,\n                    folders = appData.noteFolders,\n                    highlightQuery = searchLocateQuery,\n                    initialVersionId = initialVersionId,",
    )?;
    Ok(source)
}

fn render_smartisan_note(source: &str) -> Result<String, String> {
    let source = replace_once(
        source.to_string(),
        "internal fun SmartisanNoteEditorContent(\n    note: NoteEntry,\n    folders: List<NoteFolder>,\n    highlightQuery: String?,",
        "internal fun SmartisanNoteEditorContent(\n    note: NoteEntry,\n    folders: List<NoteFolder>,\n    highlightQuery: String?,\n    initialVersionId: String? = null,",
    )?;
    replace_once(
        source,
        "    val initialSelectedVersionId = remember(note.id) {\n        initialVersionedNote.versionIdForSearchQuery(highlightQuery.orEmpty())\n    }",
        "    val initialSelectedVersionId = remember(note.id, initialVersionId) {\n        initialVersionId?.takeIf { requested ->\n            initialVersionedNote.versions.any { version -> version.id == requested && version.deletedAtEpochMillis == null }\n        } ?: initialVersionedNote.versionIdForSearchQuery(highlightQuery.orEmpty())\n    }",
    )
}
