// v2.23.2.16 - Specify persisted property equivalence and inherited-property write barriers.
// Rust-owned source strings for the mobile structured-page feature.

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import org.junit.Assert.*
import org.junit.Test

class StructuredEditPolicyTest {
    @Test fun ordinaryDocumentCanEdit() { assertTrue(structuredEditAllowed(true,false,false,false,false,true)) }
    @Test fun encryptedIncludingUnlockedCopiesCannotEdit() { assertFalse(structuredEditAllowed(true,false,true,false,false,true)) }
    @Test fun deletedDocumentCannotEdit() { assertFalse(structuredEditAllowed(true,true,false,false,false,true)) }
    @Test fun lockedPageCannotEdit() { assertFalse(structuredEditAllowed(true,false,false,true,false,true)) }
    @Test fun stickyCannotEdit() { assertFalse(structuredEditAllowed(false,false,false,false,false,true)) }
    @Test fun unsupportedVersionCannotEdit() { assertFalse(structuredEditAllowed(true,false,false,false,false,false)) }
    @Test fun richTextCannotBeFlattened() { assertFalse(structuredEditAllowed(true,false,false,false,true,true)) }
    private fun patch() = StructuredNotePatch("n", 10L, "text", expectedValue="旧文", value="新文")
    @Test fun patchComparesTargetAndRevision() { assertTrue(structuredPatchCurrent("n",10L,"旧文",patch())) }
    @Test fun changedRevisionIsRejectedEvenWithSameText() { assertFalse(structuredPatchCurrent("n",11L,"旧文",patch())) }
    @Test fun changedTargetIsRejectedEvenWithSameRevision() { assertFalse(structuredPatchCurrent("n",10L,"新文",patch())) }
    @Test fun deletedTargetIsRejected() { assertFalse(structuredPatchCurrent("n",10L,null,patch())) }
    @Test fun anotherPageCannotReceivePatch() { assertFalse(structuredPatchCurrent("other",10L,"旧文",patch())) }
    @Test fun numericInputAcceptsFiniteValues() { for (s in listOf("0","-1.25"," 2e3 ")) assertNull(structuredInputError("number",s)) }
    @Test fun numericInputRejectsNonFiniteAndBlank() { for (s in listOf("NaN","Infinity","1e999","","abc")) assertNotNull(structuredInputError("number",s)) }
    @Test fun booleanInputIsStrict() { assertNull(structuredInputError("checkbox","true")); assertNotNull(structuredInputError("checkbox","yes")) }
    @Test fun datesAndRangesAcceptLeapDayAndEmpty() { for (s in listOf("","2024-02-29","2026-01-01 / 2026-12-31")) assertTrue(structuredDateValid(s)) }
    @Test fun datesRejectInvalidAndReversedValues() { for (s in listOf("2025-02-29","2026-10-09/2026-10-08","x","2026-10-06/")) assertFalse(structuredDateValid(s)) }
    @Test fun URLsNeverExecuteActiveSchemesOrCredentials() { for (s in listOf("javascript:alert(1)","file:///etc/passwd","https://user:pass@example.com","data:text/html,x")) assertNotNull(structuredInputError("url",s)) }
    @Test fun URLsAcceptHttpHttpsOrEmpty() { for (s in listOf("","https://example.com/a?q=x","http://localhost/x")) assertNull(structuredInputError("url",s)) }
    @Test fun tagsAreTrimmedAndDeduplicated() { assertEquals(listOf("写作","人物"),structuredTags(" 写作，人物\n写作,, ")) }
    @Test fun tagsAndTextHaveExplicitBounds() { assertNotNull(structuredInputError("tags",List(65){"t$it"}.joinToString(","))); assertNotNull(structuredInputError("text","x".repeat(100001))) }
    private val nodes=listOf(StructuredOutlineNode("p",kind="toggle"),StructuredOutlineNode("c","p","toggle"),StructuredOutlineNode("h","c","heading2","标题"),StructuredOutlineNode("z"))
    @Test fun collapsedParentHidesOnlyItsDescendants() { assertEquals(listOf("p","z"),structuredVisibleNodes(nodes,setOf("p")).map{it.id}) }
    @Test fun nestedCollapseDoesNotHideItsParent() { assertEquals(listOf("p","c","z"),structuredVisibleNodes(nodes,setOf("c")).map{it.id}) }
    @Test fun outlineOpensAllCollapsedAncestorsButKeepsOtherChoices() { assertEquals(setOf("other"),structuredExpandTo(nodes,"h",setOf("p","c","other"))) }
    @Test fun unrelatedMissingCollapsedIdDoesNotHideText() { assertEquals(nodes,structuredVisibleNodes(nodes,setOf("missing"))) }
    @Test fun missingParentsStayVisible() { val n=listOf(StructuredOutlineNode("x","absent"));assertEquals(n,structuredVisibleNodes(n,setOf("absent"))) }
    @Test fun cyclesStayVisibleAndTerminate() { val n=listOf(StructuredOutlineNode("a","b","toggle"),StructuredOutlineNode("b","a","toggle"));assertEquals(n,structuredVisibleNodes(n,setOf("a","b"))) }
    @Test fun depthIsBoundedForPhoneIndentation() { val n=(0..20).map{StructuredOutlineNode("$it",if(it==0) "" else "${it-1}")}; assertEquals(8,structuredNodeDepths(n)["20"]) }
    @Test fun saveGateBlocksDoubleClick() { val g=StructuredSaveGate();assertNotNull(g.begin());assertNull(g.begin()) }
    @Test fun oldCallbacksCannotReleaseAnotherWrite() { val g=StructuredSaveGate();val a=g.begin()!!;assertTrue(g.finish(a,true));val b=g.begin()!!;assertFalse(g.finish(a,true));assertNull(g.begin());assertTrue(g.finish(b,true)) }
    @Test fun closedAndForeignWorkspaceCallbacksCannotPublish() { val g=StructuredSaveGate();val a=g.begin()!!;g.close();assertFalse(g.finish(a,true));assertNull(g.begin());val h=StructuredSaveGate();val b=h.begin()!!;assertFalse(h.finish(b,false));assertNotNull(h.begin()) }
    @Test fun titleMustBeNonblankAndSingleLine() { assertNull(structuredInputError("title","项目")); for(s in listOf("", "a\nb", "x".repeat(201))) assertNotNull(structuredInputError("title",s)) }
}
"####;

pub const EDIT_TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import org.junit.Assert.*
import org.junit.Test
import kotlinx.serialization.json.*

class StructuredNoteEditingTest {
    private fun note() = buildStructuredMobileNote("folder",123L)
    private fun withProperty(n: NoteEntry, raw: String, key: String="amount"): NoteEntry {
        val page = n.document.knowledge!!
        val properties = page["properties"]!!.jsonObject
        return n.copy(document=n.document.copy(knowledge=JsonObject(page +
            ("properties" to JsonObject(properties + (key to Json.parseToJsonElement(raw)))))))
    }
    private fun request(n: NoteEntry, action: String, blockId: String="", key: String="", row: Int=-1, column: Int=-1, value: String="", parentId: String=""): StructuredNotePatch {
        val p=StructuredNotePatch(n.id,n.updatedAtEpochMillis,action,blockId,key,row,column,value=value,parentId=parentId)
        return p.copy(expectedValue=structuredTargetToken(n,p)!!)
    }
    @Test fun templateIsADocumentWithFolderAndNestedContent() { val n=note();assertEquals(NoteEntryKind.DOCUMENT,n.kind);assertEquals("folder",n.folderId);assertTrue(n.document.blocks.any{it.knowledge.structuredText("parentId").isNotEmpty()}) }
    @Test fun textPatchPreservesOtherBlocksAndAllPageMetadata() { val n=note();val b=n.document.blocks.first();val p=request(n,"text",b.id,value="新标题");val next=applyStructuredPatch(n,p)!!;assertEquals("新标题",next.document.blocks.first().text);assertEquals(n.document.knowledge,next.document.knowledge);assertEquals(n.document.blocks.drop(1),next.document.blocks.drop(1));assertEquals(n.attachments,next.attachments);assertEquals(n.versions,next.versions) }
    @Test fun checkboxPatchPreservesTheNodeAndParentMetadata() { val n=note();val b=n.document.blocks.first{it.knowledge.structuredText("kind")=="todo"};val next=applyStructuredPatch(n,request(n,"checked",b.id,value="true"))!!;assertTrue(next.document.blocks.first{it.id==b.id}.knowledge.structuredFlag("checked"));assertEquals(n.document.blocks.map{it.id},next.document.blocks.map{it.id}) }
    @Test fun tableCellPatchChangesOnlyRequestedCell() { val n=note();val b=n.document.blocks.last();val next=applyStructuredPatch(n,request(n,"cell",b.id,row=1,column=1,value="说明"))!!;val t=next.document.blocks.last().knowledge!!["table"] as JsonArray;assertEquals("说明",((t[1] as JsonArray)[1] as JsonPrimitive).content);assertEquals(b.knowledge!!["table"]!!.jsonArray[0],t[0]) }
    @Test fun tableCanAppendRowAndColumnWithoutDiscardingExistingValues() { val n=note();val b=n.document.blocks.last();val row=applyStructuredPatch(n,request(n,"table_row",b.id))!!;assertEquals(3,row.document.blocks.last().knowledge!!["table"]!!.jsonArray.size);val col=applyStructuredPatch(row,request(row,"table_column",b.id))!!;assertEquals(3,col.document.blocks.last().knowledge!!["table"]!!.jsonArray[0].jsonArray.size) }
    @Test fun existingPropertyPatchPreservesTypeAndOtherProperties() { val n=note();val next=applyStructuredPatch(n,request(n,"property",key="status",value="进行中"))!!;val props=next.document.knowledge!!["properties"]!!.jsonObject;assertEquals("text",props["status"]!!.jsonObject.structuredText("kind"));assertEquals("进行中",props["status"]!!.jsonObject.structuredText("value"));assertEquals(n.document.knowledge!!["properties"]!!.jsonObject["date"],props["date"]) }
    @Test fun tagsAreStoredAsAnArrayNotJoinedText() { val n=note();val next=applyStructuredPatch(n,request(n,"tags",value="人物\n写作\n人物"))!!;assertEquals(2,next.document.knowledge!!["tags"]!!.jsonArray.size) }
    @Test fun childInsertionPreservesExistingBlocksAndLinksItsParent() { val n=note();val parent=n.document.blocks.first{it.knowledge.structuredText("kind")=="toggle"};val p=request(n,"append","new",key="paragraph",value="子项",parentId=parent.id);val next=applyStructuredPatch(n,p)!!;assertEquals(n.document.blocks,next.document.blocks.filterNot{it.id=="new"});assertEquals(parent.id,next.document.blocks.first{it.id=="new"}.knowledge.structuredText("parentId")) }
    @Test fun stalePatchNeverOverwritesAnyField() { val n=note();val p=request(n,"text",n.document.blocks.first().id,value="new");assertNull(applyStructuredPatch(n.copy(updatedAtEpochMillis=124L),p));assertNull(applyStructuredPatch(n.copy(deletedAtEpochMillis=1L),p)) }
    @Test fun protectedPagesAndRichTextCannotBeChanged() { val n=note();val p=request(n,"tags",value="private");assertNull(applyStructuredPatch(n.copy(encryption=NoteEncryptionEnvelope(),encryptionUnlocked=true),p));assertNull(applyStructuredPatch(n.copy(document=n.document.copy(richTextEnabled=true)),p));assertNull(applyStructuredPatch(n.copy(document=n.document.copy(knowledge=JsonObject(n.document.knowledge!!+("locked" to JsonPrimitive(true))))),p)) }
    @Test fun unknownPropertyTypesAndBlockKindsRemainUnchanged() { val n=note();val page=n.document.knowledge!!;val props=page["properties"]!!.jsonObject;val modified=n.copy(document=n.document.copy(knowledge=JsonObject(page+("properties" to JsonObject(props+("status" to buildJsonObject{put("kind","relation");put("value",JsonArray(emptyList()))}))))));assertNull(applyStructuredPatch(modified,request(modified,"property",key="status",value="x"))) }
    @Test fun verifiedWriteChecksTheActualFieldNotJustSuccessStatus() { val n=note();val p=request(n,"text",n.document.blocks.first().id,value="new");val next=applyStructuredPatch(n,p)!!;val expected=structuredTargetToken(next,p);assertTrue(structuredPatchVerified(next,p,expected));assertFalse(structuredPatchVerified(n,p,expected)) }
    @Test fun missingCheckboxCannotBecomeASuccessfulNoop() { val n=note();val p=StructuredNotePatch(n.id,n.updatedAtEpochMillis,"checked","missing",expectedValue="false",value="true");assertNull(structuredTargetToken(n,p));assertNull(applyStructuredPatch(n,p)) }
    @Test fun pageTitleEditDoesNotChangeBodyOrProperties() { val n=note();val next=applyStructuredPatch(n,request(n,"title",value="我的项目"))!!;assertEquals("我的项目",next.title);assertEquals(n.document,next.document) }
    @Test fun propertyTokenIgnoresObjectOrderAndEquivalentDecimalNotation() {
        val original=withProperty(note(),"""{"kind":"number","value":1e-7,"meta":{"a":1,"b":2}}""")
        val reordered=withProperty(original,"""{"meta":{"b":2.0,"a":1.0},"value":1.0E-7,"kind":"number"}""")
        val patch=request(original,"property",key="amount",value="0.25")
        assertEquals(patch.expectedValue,structuredTargetToken(reordered,patch))
        assertNotNull(applyStructuredPatch(reordered,patch))
    }
    @Test fun numericPersistedVerificationAcceptsRustF64Notation() {
        for (value in listOf("1e-7","1e23")) {
            val original=withProperty(note(),"""{"kind":"number","value":0.0}""")
            val patch=request(original,"property",key="amount",value=value)
            val edited=applyStructuredPatch(original,patch)!!
            val expected=structuredTargetToken(edited,patch)
            val persisted=withProperty(edited,"""{"value":$value,"kind":"number"}""")
            assertTrue(value,structuredPatchVerified(persisted,patch,expected))
        }
    }
    @Test fun numericVerificationRejectsDifferentValuesAndJsonTypes() {
        val original=withProperty(note(),"""{"kind":"number","value":0.0}""")
        val patch=request(original,"property",key="amount",value="1e-7")
        val edited=applyStructuredPatch(original,patch)!!
        val expected=structuredTargetToken(edited,patch)
        for (cell in listOf("""{"kind":"number","value":1.1e-7}""",
                """{"kind":"number","value":"1e-7"}""", """{"kind":"text","value":1e-7}""")) {
            assertFalse(cell,structuredPatchVerified(withProperty(edited,cell),patch,expected))
        }
    }
    @Test fun propertyCasDoesNotRoundDistinctLargeIntegersIntoTheSameTarget() {
        val original=withProperty(note(),"""{"kind":"number","value":9007199254740992}""")
        val changed=withProperty(original,"""{"kind":"number","value":9007199254740993}""")
        val patch=request(original,"property",key="amount",value="2")
        assertNotEquals(patch.expectedValue,structuredTargetToken(changed,patch))
        assertNull(applyStructuredPatch(changed,patch))
    }
    @Test fun numericVerificationKeepsAllOtherMetadataExact() {
        val original=withProperty(note(),"""{"kind":"number","value":0.0,"meta":{"revision":9007199254740992}}""")
        val patch=request(original,"property",key="amount",value="1e-7")
        val edited=applyStructuredPatch(original,patch)!!
        val expected=structuredTargetToken(edited,patch)
        val persisted=withProperty(edited,"""{"meta":{"revision":9007199254740992},"value":1e-7,"kind":"number"}""")
        assertTrue(structuredPatchVerified(persisted,patch,expected))
        val changed=withProperty(edited,"""{"meta":{"revision":9007199254740993},"value":1e-7,"kind":"number"}""")
        assertFalse(structuredPatchVerified(changed,patch,expected))
    }
    @Test fun numericVerificationPreservesTheF64SignedZero() {
        val original=withProperty(note(),"""{"kind":"number","value":0.0}""")
        val patch=request(original,"property",key="amount",value="-0.0")
        val edited=applyStructuredPatch(original,patch)!!
        val expected=structuredTargetToken(edited,patch)
        assertTrue(structuredPatchVerified(withProperty(edited,"""{"kind":"number","value":-0.0}"""),patch,expected))
        assertFalse(structuredPatchVerified(withProperty(edited,"""{"kind":"number","value":0.0}"""),patch,expected))
        assertNotEquals(patch.expectedValue,structuredTargetToken(edited,patch))
    }
    @Test fun ownDatabaseAndInheritedPropertiesAreReadOnly() {
        val original=note()
        val patch=request(original,"property",key="status",value="进行中")
        for (metadata in listOf("parentId" to JsonPrimitive("database"),"database" to buildJsonObject {})) {
            val page=JsonObject(original.document.knowledge!! + metadata)
            assertFalse(page.structuredPropertiesEditable())
            assertNull(applyStructuredPatch(original.copy(document=original.document.copy(knowledge=page)),patch))
        }
        assertTrue(JsonObject(original.document.knowledge!! + mapOf("parentId" to JsonNull,"database" to JsonNull)).structuredPropertiesEditable())
    }
    @Test fun inheritedPropertyProtectionKeepsBodyAndTodoEditingAvailable() {
        val base=note()
        val original=base.copy(document=base.document.copy(knowledge=JsonObject(base.document.knowledge!! + ("parentId" to JsonPrimitive("database")))))
        val body=original.document.blocks.first()
        val text=applyStructuredPatch(original,request(original,"text",body.id,value="新正文"))!!
        assertEquals("新正文",text.document.blocks.first().text)
        assertEquals(original.document.knowledge,text.document.knowledge)
        val todo=original.document.blocks.first{it.knowledge.structuredText("kind")=="todo"}
        val checked=applyStructuredPatch(original,request(original,"checked",todo.id,value="true"))!!
        assertTrue(checked.document.blocks.first{it.id==todo.id}.knowledge.structuredFlag("checked"))
        assertEquals(original.document.knowledge,checked.document.knowledge)
    }
}
"####;
