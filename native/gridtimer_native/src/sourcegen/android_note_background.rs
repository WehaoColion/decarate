// v2.22.49.2 - Invalidate pending encrypted-note unlocks on background and disposal.
// v2.22.44 - Preserve ordinary note pages and cancel stale background lock jobs.
pub const PATH: &str = "com/ofairyo/gridtimer/ui/NoteBackgroundLockObserver.kt";
const SCREEN: &str = "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    if path != SCREEN {
        return Ok(base.to_owned());
    }
    if base.matches(OLD_OBSERVER).count() != 2 {
        return Err("expected notebook and sticky-note background lock observers".to_owned());
    }
    Ok(base.replace(OLD_OBSERVER, NEW_OBSERVER))
}

const OLD_OBSERVER: &str = r####"    val latestUnlockedNote by rememberUpdatedState(unlockedNote)
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) {
                scope.launch {
                    delay(700L)
                    latestUnlockedNote?.id?.let(viewModel::lockEncryptedNote)
                    unlockedNote = null
                    selectedNoteId = null
                }
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }"####;

const NEW_OBSERVER: &str = r####"    val latestUnlockedNote by rememberUpdatedState(unlockedNote)
    val latestSelectedNoteId by rememberUpdatedState(selectedNoteId)
    DisposableEffect(lifecycleOwner, viewModel) {
        val observer = NoteBackgroundLockObserver(
            scope = scope,
            unlockedNoteId = { latestUnlockedNote?.id },
            onBackground = viewModel::cancelPendingNoteUnlocks,
            onLock = { lockedNoteId ->
                viewModel.lockEncryptedNote(lockedNoteId)
                unlockedNote = null
                if (latestSelectedNoteId == lockedNoteId) {
                    attachmentPreview = null
                    selectedNoteId = null
                }
            }
        )
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            viewModel.cancelPendingNoteUnlocks()
            observer.dispose()
            lifecycleOwner.lifecycle.removeObserver(observer)
        }
    }"####;

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import com.ofairyo.gridtimer.data.NoteEntry

// The delay lets the editor flush before the encryption session is closed.
// An ordinary document has no session to lock and must keep its navigation state.
internal class NoteBackgroundLockObserver(
    private val scope: CoroutineScope,
    private val unlockedNoteId: () -> String?,
    private val onLock: (String) -> Unit,
    private val awaitTimeout: suspend () -> Unit = { delay(700L) },
    private val onBackground: () -> Unit = {}
) : LifecycleEventObserver {
    private var pendingLock: Job? = null
    private var stopped = false

    override fun onStateChanged(source: LifecycleOwner, event: Lifecycle.Event) {
        when (event) {
            Lifecycle.Event.ON_STOP -> {
                onBackground()
                cancelPendingLock()
                stopped = true
                val noteId = unlockedNoteId() ?: return
                pendingLock = scope.launch {
                    awaitTimeout()
                    if (stopped && unlockedNoteId() == noteId) {
                        onLock(noteId)
                    }
                }
            }
            Lifecycle.Event.ON_START, Lifecycle.Event.ON_DESTROY -> cancelPendingLock()
            else -> Unit
        }
    }

    fun dispose() = cancelPendingLock()

    private fun cancelPendingLock() {
        stopped = false
        pendingLock?.cancel()
        pendingLock = null
    }
}

// Only a still-current request may publish decrypted content. Identity tickets
// also reject a request superseded by a second unlock of the same document.
internal class NoteUnlockRequestGate {
    class Ticket internal constructor(val workspaceKey: String, val noteId: String)
    private val pending = mutableMapOf<Pair<String, String>, Ticket>()

    @Synchronized
    fun begin(workspaceKey: String, noteId: String): Ticket = Ticket(workspaceKey, noteId).also {
        pending[workspaceKey to noteId] = it
    }

    @Synchronized
    fun finish(ticket: Ticket, currentWorkspaceKey: String, unlockedNoteId: String?): Boolean {
        val key = ticket.workspaceKey to ticket.noteId
        if (pending[key] !== ticket) return false
        pending.remove(key)
        return currentWorkspaceKey == ticket.workspaceKey && unlockedNoteId == ticket.noteId
    }

    @Synchronized
    fun discard(ticket: Ticket) {
        val key = ticket.workspaceKey to ticket.noteId
        if (pending[key] === ticket) pending.remove(key)
    }

    @Synchronized
    fun invalidate(workspaceKey: String, noteId: String) {
        pending.remove(workspaceKey to noteId)
    }

    @Synchronized
    fun invalidateAll() = pending.clear()
}

internal fun noteUnlockTargetMatches(requested: NoteEntry, current: NoteEntry?): Boolean =
    requested.encryption != null && current != null && current.id == requested.id &&
        current.deletedAtEpochMillis == null && current.encryption == requested.encryption &&
        current.protectionStateRevision == requested.protectionStateRevision
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_note_surfaces_use_the_background_lock_guard() {
        let studio = super::super::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt")
            .unwrap();
        let rendered = render(studio.path, studio.contents).unwrap();
        assert_eq!(
            rendered
                .matches("val observer = NoteBackgroundLockObserver(")
                .count(),
            2
        );
        assert!(!rendered.contains(OLD_OBSERVER));
    }
}
