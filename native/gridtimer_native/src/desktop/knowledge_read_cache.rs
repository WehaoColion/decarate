// Windows - Reuse workspace navigation search and database row indices across warm frames.
// v1.0.3 Windows - Bound and invalidate gallery previews with the workspace read cache.
// v1.0.3 Windows - Drop history read models with the workspace and privacy cache.
// v1.0.2.1 Windows - Cache flattened page navigation for viewport-only rendering.
// v2.22.54 - Bounded, versioned read models keep document history out of frame rendering.
const KNOWLEDGE_GALLERY_PREVIEW_LIMIT: usize = 4096;

#[derive(Default)]
struct KnowledgeReadCache {
    version: Option<u64>,
    navigation: Option<Arc<KnowledgeNavigation>>,
    note_indices: Option<Arc<BTreeMap<String, usize>>>,
    tree_rows: Option<(HashSet<String>, Arc<Vec<KnowledgeTreeRow>>)>,
    records: Option<Arc<Vec<knowledge::PageRecord>>>,
    record_indices: Option<Arc<std::collections::HashMap<String, usize>>>,
    record_search: Option<(Arc<Vec<String>>, String, Arc<Vec<String>>)>,
    quick_choices: Option<KnowledgeQuickChoiceCache>,
    gallery_previews: std::collections::HashMap<String, (bool, Arc<String>)>,
    rich_preview: Option<(String, Arc<String>)>,
    history_rows: Option<(String, Arc<Vec<KnowledgeHistoryRow>>)>,
    history_comparison: Option<(
        KnowledgeHistoryComparisonKey,
        Arc<KnowledgeHistoryComparison>,
    )>,
}
impl KnowledgeReadCache {
    fn at_version(&mut self, version: u64) {
        if self.version != Some(version) {
            *self = Self {
                version: Some(version),
                ..Default::default()
            };
        }
    }
}
struct KnowledgeNavigationPage {
    id: String,
    title: String,
    parent_id: Option<String>,
    order: i64,
    icon: String,
    pinned: bool,
    encrypted: bool,
    locked: bool,
    database: bool,
    updated_at_epoch_millis: i64,
}
#[derive(Default)]
struct KnowledgeNavigation {
    pages: Vec<KnowledgeNavigationPage>,
    by_id: BTreeMap<String, usize>,
    roots: Vec<usize>,
    children: BTreeMap<String, Vec<usize>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KnowledgeTreeRow {
    PinnedHeading,
    Pinned(usize),
    PagesHeading,
    Page { index: usize, depth: usize },
}
impl KnowledgeNavigation {
    fn from_notes(notes: &[DesktopNote]) -> Self {
        let mut result = Self::default();
        for note in notes.iter().filter(|note| {
            desktop_note_kind(note) == DesktopNoteKind::Document
                && note.deleted_at_epoch_millis.is_none()
        }) {
            let meta = note.document.knowledge.as_ref();
            let index = result.pages.len();
            let parent_id = meta.and_then(|m| m.parent_id.clone());
            if let Some(parent) = &parent_id {
                result
                    .children
                    .entry(parent.clone())
                    .or_default()
                    .push(index);
            } else {
                result.roots.push(index);
            }
            result.by_id.insert(note.id.clone(), index);
            result.pages.push(KnowledgeNavigationPage {
                id: note.id.clone(),
                title: note.title.clone(),
                parent_id,
                order: meta.map_or(0, |m| m.order),
                icon: meta.map(|m| m.icon.clone()).unwrap_or_default(),
                pinned: note.pinned,
                encrypted: note.encryption.is_some(),
                locked: meta.is_some_and(|m| m.locked),
                database: meta.is_some_and(|m| m.database.is_some()),
                updated_at_epoch_millis: note.updated_at_epoch_millis,
            });
        }
        let pages = &result.pages;
        let compare = |a: &usize, b: &usize| {
            pages[*a]
                .order
                .cmp(&pages[*b].order)
                .then_with(|| pages[*a].title.cmp(&pages[*b].title))
                .then_with(|| pages[*a].id.cmp(&pages[*b].id))
        };
        result.roots.sort_by(compare);
        for children in result.children.values_mut() {
            children.sort_by(compare);
        }
        result
    }
    fn children_of(&self, parent: Option<&str>) -> &[usize] {
        match parent {
            None => &self.roots,
            Some(id) => self.children.get(id).map(Vec::as_slice).unwrap_or_default(),
        }
    }
    fn tree_rows(&self, collapsed: &HashSet<String>) -> Vec<KnowledgeTreeRow> {
        let mut rows = Vec::with_capacity(self.pages.len() + 2);
        if self.pages.iter().any(|page| page.pinned) {
            rows.push(KnowledgeTreeRow::PinnedHeading);
            rows.extend(
                self.pages
                    .iter()
                    .enumerate()
                    .filter(|(_, page)| page.pinned)
                    .map(|(index, _)| KnowledgeTreeRow::Pinned(index)),
            );
        }
        rows.push(KnowledgeTreeRow::PagesHeading);
        let mut seen = vec![false; self.pages.len()];
        for &index in &self.roots {
            self.append_tree_rows(index, 0, collapsed, &mut seen, &mut rows);
        }
        // Keep orphaned pages and malformed cycles reachable after sync/trash.
        // Descendants of an explicitly collapsed page are already marked seen.
        for index in 0..self.pages.len() {
            if !seen[index] {
                self.append_tree_rows(index, 0, collapsed, &mut seen, &mut rows);
            }
        }
        rows
    }
    fn append_tree_rows(
        &self,
        index: usize,
        depth: usize,
        collapsed: &HashSet<String>,
        seen: &mut [bool],
        rows: &mut Vec<KnowledgeTreeRow>,
    ) {
        if depth > 32 || seen[index] {
            return;
        }
        seen[index] = true;
        rows.push(KnowledgeTreeRow::Page { index, depth });
        if collapsed.contains(&self.pages[index].id) {
            let mut hidden = vec![index];
            while let Some(parent) = hidden.pop() {
                for &child in self.children_of(Some(&self.pages[parent].id)) {
                    if !seen[child] {
                        seen[child] = true;
                        hidden.push(child);
                    }
                }
            }
        } else {
            for &child in self.children_of(Some(&self.pages[index].id)) {
                self.append_tree_rows(child, depth + 1, collapsed, seen, rows);
            }
        }
    }
    fn ancestors(&self, id: &str) -> Vec<&KnowledgeNavigationPage> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let mut current = self.by_id.get(id).map(|i| &self.pages[*i]);
        seen.insert(id);
        while let Some(parent) = current.and_then(|p| p.parent_id.as_deref()) {
            if !seen.insert(parent) {
                break;
            }
            current = self
                .by_id
                .get(parent)
                .map(|i| &self.pages[*i])
                .filter(|page| !page.encrypted);
            if let Some(page) = current {
                result.push(page);
            }
        }
        result.reverse();
        result
    }
}
#[derive(PartialEq)]
struct KnowledgeQueryKey {
    version: u64,
    page_id: String,
    today: i64,
    database: knowledge::KnowledgeDatabase,
    view: knowledge::DatabaseView,
    meta: Option<knowledge::KnowledgePage>,
}
impl TimerWindowsClient {
    fn invalidate_knowledge_read_cache(&self) {
        *self.desktop_ui.knowledge.read_cache.borrow_mut() = KnowledgeReadCache::default();
    }
    fn knowledge_note_indices(&self) -> Arc<BTreeMap<String, usize>> {
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        Arc::clone(cache.note_indices.get_or_insert_with(|| {
            Arc::new(
                self.data
                    .notes
                    .iter()
                    .enumerate()
                    .map(|(index, note)| (note.id.clone(), index))
                    .collect(),
            )
        }))
    }

    fn knowledge_navigation(&self) -> Arc<KnowledgeNavigation> {
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        cache
            .navigation
            .get_or_insert_with(|| Arc::new(KnowledgeNavigation::from_notes(&self.data.notes)))
            .clone()
    }
    fn knowledge_tree_rows(&self, navigation: &KnowledgeNavigation) -> Arc<Vec<KnowledgeTreeRow>> {
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        let collapsed = &self.desktop_ui.knowledge.collapsed;
        if !cache
            .tree_rows
            .as_ref()
            .is_some_and(|(previous, _)| previous == collapsed)
        {
            cache.tree_rows = Some((collapsed.clone(), Arc::new(navigation.tree_rows(collapsed))));
        }
        Arc::clone(&cache.tree_rows.as_ref().unwrap().1)
    }
    fn cached_rich_note_preview(&self) -> Arc<String> {
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        if !cache
            .rich_preview
            .as_ref()
            .is_some_and(|(id, _)| id == &self.selected_note_id)
        {
            let text = self
                .selected_note_ref()
                .map(desktop_note_rich_text_plain_text)
                .unwrap_or_default();
            cache.rich_preview = Some((self.selected_note_id.clone(), Arc::new(text)));
        }
        cache.rich_preview.as_ref().unwrap().1.clone()
    }

    fn cached_knowledge_gallery_preview(&self, page: &knowledge::PageRecord) -> Arc<String> {
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        // Never reuse an earlier plaintext preview when a record becomes protected.
        if page.encrypted || page.deleted {
            cache.gallery_previews.remove(&page.id);
            return Arc::new(String::new());
        }
        if let Some((locked, text)) = cache.gallery_previews.get(&page.id) {
            if *locked == page.meta.locked {
                return Arc::clone(text);
            }
        }
        let text = Arc::new(knowledge_gallery_preview_text(&page.content));
        if cache.gallery_previews.len() < KNOWLEDGE_GALLERY_PREVIEW_LIMIT
            || cache.gallery_previews.contains_key(&page.id)
        {
            cache
                .gallery_previews
                .insert(page.id.clone(), (page.meta.locked, Arc::clone(&text)));
        }
        text
    }
}
