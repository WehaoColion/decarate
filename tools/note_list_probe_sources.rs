// v0.0.1 - JVM probe for the exact generated note query bodies.
pub const DRIVER: &str = r###"package perf
import com.ofairyo.gridtimer.data.*
import perf.before.activeNotes as previousActive
import perf.before.sortedActiveNotebookDocuments as previousDocuments
import perf.current.activeNotes as optimizedActive
import perf.current.sortedActiveNotebookDocuments as optimizedDocuments
import perf.current.noteSortTitles as optimizedTitles
import java.lang.management.ManagementFactory
import kotlinx.serialization.json.*

fun main() {
    val bean=ManagementFactory.getThreadMXBean() as com.sun.management.ThreadMXBean
    check(bean.isThreadAllocatedMemorySupported); bean.isThreadAllocatedMemoryEnabled=true
    val thread=Thread.currentThread().id
    val notes=List(2000) { NoteEntry(id="n$it",title="Title $it",content="sample content ".repeat(80),kind=if(it%2==0) NoteEntryKind.DOCUMENT else NoteEntryKind.STICKY,deletedAtEpochMillis=if(it%10==0) 1L else null,createdAtEpochMillis=it.toLong(),updatedAtEpochMillis=(2000-it).toLong()) }
    val data=AppData(notes=notes,syncConflictHistory=List(300){SyncConflictRecord(id="history$it",payload=JsonPrimitive("history ".repeat(1024)))})
    val expectedVisible=data.notes.filterNot(NoteEntry::isDeleted).map { it.id }
    val expectedDocuments=expectedVisible.filter { it.removePrefix("n").toInt()%2==0 }
    val cases=listOf("before_visibility" to { data.previousActive() },"after_visibility" to { data.optimizedActive() },"before_documents" to { data.previousDocuments() },"after_documents" to { data.optimizedDocuments() })
    val results=buildJsonArray {
        for ((name,operation) in cases) {
            repeat(8) { operation() }
            val samples=buildJsonArray {
                repeat(20) {
                    val beforeBytes=bean.getThreadAllocatedBytes(thread); val started=System.nanoTime()
                    val result=operation(); val nanos=System.nanoTime()-started
                    val allocated=bean.getThreadAllocatedBytes(thread)-beforeBytes
                    check(result.map { it.id } == if(name.endsWith("documents")) expectedDocuments else expectedVisible)
                    add(buildJsonObject { put("nanos",nanos); put("allocatedBytes",allocated) })
                }
            }
            add(buildJsonObject { put("case",name); put("samples",samples) })
        }
    }
    val titleReads=buildJsonObject {
        for(mode in NoteSortMode.values()) {
            var calls=0
            optimizedTitles(notes,mode) { calls++; it.title }
            check(calls == if(mode==NoteSortMode.CREATED_ASC || mode==NoteSortMode.TITLE_ASC) notes.size else 0)
            put(mode.name,calls)
        }
    }
    println(buildJsonObject { put("passed",true); put("notes",notes.size); put("unrelatedHistoryEntries",300); put("hostOnly",true); put("nativeBridge","unavailable on JVM; prior serialization still executed, native parsing excluded"); put("results",results); put("optimizedTitleReads",titleReads) })
}
"###;
