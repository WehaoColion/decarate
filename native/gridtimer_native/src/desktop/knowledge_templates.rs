// v2.22.52 - Built-in templates create a usable page hierarchy in one transaction.

fn knowledge_template_pages(template: KnowledgeTemplate, folder: Option<&str>) -> Vec<Value> {
    let root = random_desktop_identifier("page");
    let database = random_desktop_identifier("database");
    let (title, icon, body, database_name) = match template {
        KnowledgeTemplate::Project => ("新项目", "◈", "## 目标\n\n## 范围\n\n## 交付标准\n- [ ] 完成交付检查\n\n## 里程碑\n| 阶段 | 日期 | 交付物 |\n| --- | --- | --- |\n| 第一阶段 | | |\n\n## 决策记录\n", Some("任务清单")),
        KnowledgeTemplate::Meeting => ("会议记录", "☷", "## 会议信息\n| 项目 | 内容 |\n| --- | --- |\n| 时间 | |\n| 参与人 | |\n\n## 议题\n\n## 讨论记录\n\n## 决议\n\n## 待确认\n- [ ] \n", Some("会议行动项")),
        KnowledgeTemplate::Reading => ("阅读笔记", "▤", "## 书目信息\n| 项目 | 内容 |\n| --- | --- |\n| 书名或文章 | |\n| 作者 | |\n| 来源 | |\n\n## 核心观点\n\n## 摘录\n> \n\n## 我的理解\n\n## 待研究的问题\n- [ ] \n", None),
        KnowledgeTemplate::Review => ("阶段复盘", "↻", "## 本轮目标与结果\n| 目标 | 结果 | 差异 |\n| --- | --- | --- |\n| | | |\n\n## 有效做法\n\n## 问题与原因\n\n## 保留、停止、尝试\n| 保留 | 停止 | 尝试 |\n| --- | --- | --- |\n| | | |\n", Some("改进行动")),
        KnowledgeTemplate::Tasks => ("任务管理", "▦", "", None),
        KnowledgeTemplate::Blank => ("未命名", "", "", None),
    };
    let mut page = knowledge::KnowledgePage {
        icon: icon.into(),
        ..Default::default()
    };
    if template == KnowledgeTemplate::Tasks {
        page.database = Some(knowledge::KnowledgeDatabase::task_database());
    }
    let mut blocks = knowledge::markdown_blocks(body, new_desktop_note_block_id);
    if let Some(name) = database_name {
        blocks.push(json!({"id":new_desktop_note_block_id(),"type":"TEXT","text":name,"knowledge":{"kind":"page_link","targetPageId":database}}));
    }
    let mut pages = vec![
        json!({"id":root,"kind":"DOCUMENT","title":title,"folderId":folder,"document":{"knowledge":page,"blocks":blocks}}),
    ];
    if let Some(name) = database_name {
        pages.push(json!({"id":database,"kind":"DOCUMENT","title":name,"folderId":folder,"document":{"knowledge":knowledge::KnowledgePage {parent_id:Some(root),icon:"▦".into(),database:Some(knowledge::KnowledgeDatabase::task_database()),..Default::default()},"blocks":[]}}));
    }
    pages
}
