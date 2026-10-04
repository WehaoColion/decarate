//! Official provision excerpts allowed in legal clue reports on both clients.

use crate::legal_scan::{LegalReport, VerifiedLegalSource};

/// Check citations at every import boundary. A receiver cannot reconstruct the
/// original scan, but it can reject invented evidence IDs and legal provisions.
pub fn validate_report_sources(report: &LegalReport) -> Result<(), String> {
    let allowed = verified_laws();
    for finding in &report.findings {
        if finding.evidence.is_empty() {
            return Err("法律线索缺少原始记录引文".into());
        }
        for citation in &finding.evidence {
            let id = citation.evidence_id.as_bytes();
            let ordinal = id
                .get(1..)
                .and_then(|digits| std::str::from_utf8(digits).ok())
                .and_then(|digits| digits.parse::<usize>().ok());
            if id.len() < 7
                || id[0] != b'E'
                || !id[1..].iter().all(u8::is_ascii_digit)
                || !matches!(ordinal, Some(number) if number > 0 && number <= report.manifest.evidence_count)
                || citation.source_path.trim().is_empty()
                || citation.quote.trim().is_empty()
            {
                return Err("法律线索的证据编号或引文无效".into());
            }
        }
        for law in &finding.laws {
            let verified = allowed.iter().any(|item| {
                item.id == law.id
                    && item.title == law.title
                    && item.version == law.version
                    && ((item.url == law.url
                        && valid_prior_review_date(&law.checked_on, &item.checked_on))
                        || legacy_verified_url(&law.id, &law.url, &law.checked_on))
                    && item.article_number == law.article_number
                    && item.article_text == law.article_text
            });
            if !verified {
                return Err("法律线索包含未经核对的法条".into());
            }
        }
    }
    Ok(())
}

// The Android 2.23.1 release embedded these two official links. Keep the
// precise historical variants so its saved reports can migrate unchanged.
fn legacy_verified_url(id: &str, url: &str, checked_on: &str) -> bool {
    checked_on == "2026-09-29"
        && matches!(
            (id, url),
            (
                "copyright_26",
                "https://policy.mofcom.gov.cn/claw/clawContent.shtml?id=87976"
            ) | (
                "individual_income_tax_2",
                "https://www.dhlc.gov.cn/rsj/Web/_F0_0_28D05L2LRE40ALPAOCV8UNHIHZ.htm"
            )
        )
}

fn valid_prior_review_date(value: &str, latest: &str) -> bool {
    let raw = value.as_bytes();
    if raw.len() != 10
        || raw[4] != b'-'
        || raw[7] != b'-'
        || !raw
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return false;
    }
    let year = value[0..4].parse::<u16>().unwrap_or(0);
    let month = value[5..7].parse::<u8>().unwrap_or(0);
    let day = value[8..10].parse::<u8>().unwrap_or(0);
    year >= 2020 && (1..=12).contains(&month) && (1..=31).contains(&day) && value <= latest
}

pub fn verified_laws() -> Vec<VerifiedLegalSource> {
    // Recheck the linked official text and the checked_on date for each release.
    vec![
        VerifiedLegalSource {
            id: "civil_code_577".into(),
            title: "中华人民共和国民法典".into(),
            version: "2020年通过，2021年施行".into(),
            url: "https://wb.flk.npc.gov.cn/flfg/PDF/bd53dd912c1048f2aecbaa229238334b.pdf".into(),
            checked_on: "2026-09-29".into(),
            article_number: "五百七十七".into(),
            article_text: "当事人一方不履行合同义务或者履行合同义务不符合约定的，应当承担继续履行、采取补救措施或者赔偿损失等违约责任。".into(),
        },
        VerifiedLegalSource {
            id: "personal_information_protection_6".into(),
            title: "中华人民共和国个人信息保护法".into(),
            version: "2021年通过，2021年施行".into(),
            url: "https://www.cac.gov.cn/2021-08/20/c_1631050028355286.htm".into(),
            checked_on: "2026-09-29".into(),
            article_number: "六".into(),
            article_text: "收集个人信息，应当限于实现处理目的的最小范围，不得过度收集个人信息。".into(),
        },
        VerifiedLegalSource {
            id: "copyright_26".into(),
            title: "中华人民共和国著作权法".into(),
            version: "2020年修正，2021年施行".into(),
            url: "https://wb.flk.npc.gov.cn/flfg/PDF/3d00c03d87cf4863878081a3e1b54638.pdf".into(),
            checked_on: "2026-09-29".into(),
            article_number: "二十六".into(),
            article_text: "使用他人作品应当同著作权人订立许可使用合同，本法规定可以不经许可的除外。".into(),
        },
        VerifiedLegalSource {
            id: "labor_contract_10".into(),
            title: "中华人民共和国劳动合同法".into(),
            version: "2012年修正".into(),
            url: "https://policy.mofcom.gov.cn/claw/clawContent.shtml?id=4545".into(),
            checked_on: "2026-09-29".into(),
            article_number: "十".into(),
            article_text: "建立劳动关系，应当订立书面劳动合同。".into(),
        },
        VerifiedLegalSource {
            id: "individual_income_tax_2".into(),
            title: "中华人民共和国个人所得税法".into(),
            version: "2018年修正".into(),
            url: "https://jiangsu.chinatax.gov.cn/art/2018/8/31/art_23636_1794.html".into(),
            checked_on: "2026-09-29".into(),
            article_number: "二".into(),
            article_text: "下列各项个人所得，应当缴纳个人所得税： （一）工资、薪金所得； （二）劳务报酬所得；".into(),
        },
        VerifiedLegalSource {
            id: "consumer_protection_20".into(),
            title: "中华人民共和国消费者权益保护法".into(),
            version: "2013年修正".into(),
            url: "https://www.npc.gov.cn/WZWSREL3pncmR3Ly9ucGMvLy8vLy8vbGZ6dC94ZnpxeWJoZnh6YS8yMDE0LTAxLzAyL2NvbnRlbnRfMTg3MjQ4OC5odG0%3D".into(),
            checked_on: "2026-09-29".into(),
            article_number: "二十".into(),
            article_text: "经营者向消费者提供有关商品或者服务的质量、性能、用途、有效期限等信息，应当真实、全面，不得作虚假或者引人误解的宣传。".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legal_scan::{LegalEvidenceQuote, LegalFinding, LegalScanManifest};

    #[test]
    fn imported_report_rejects_unverified_laws_and_invalid_evidence() {
        let mut report = LegalReport {
            workspace_id: "source".into(),
            captured_at_epoch_millis: 1,
            completed: true,
            findings: vec![LegalFinding {
                title: "待核查".into(),
                area: "合同债务".into(),
                fact: "约定待核查".into(),
                evidence: vec![LegalEvidenceQuote {
                    evidence_id: "E000001".into(),
                    source_path: "notes/n1".into(),
                    title: "记录".into(),
                    quote: "约定".into(),
                }],
                event_at_epoch_millis: None,
                missing_facts: "合同文本".into(),
                recommendation: "核对原件".into(),
                laws: vec![verified_laws()[0].clone()],
            }],
            manifest: LegalScanManifest {
                workspace_id: "source".into(),
                captured_at_epoch_millis: 1,
                coverage: Default::default(),
                omissions: Vec::new(),
                evidence_count: 1,
                upload_bytes: 0,
                estimated_calls: 0,
            },
            errors: Vec::new(),
        };
        assert!(validate_report_sources(&report).is_ok());
        report.findings[0].laws[0].checked_on = "2026-09-28".into();
        assert!(validate_report_sources(&report).is_ok());
        report.findings[0].laws[0] = verified_laws()[2].clone();
        report.findings[0].laws[0].url =
            "https://policy.mofcom.gov.cn/claw/clawContent.shtml?id=87976".into();
        assert!(validate_report_sources(&report).is_ok());
        report.findings[0].laws[0].url = "https://example.com/forged".into();
        assert!(validate_report_sources(&report).is_err());
        report.findings[0].laws[0] = verified_laws()[0].clone();
        report.findings[0].laws[0].article_text = "伪造条文".into();
        assert!(validate_report_sources(&report).is_err());
        report.findings[0].laws.clear();
        report.findings[0].evidence[0].evidence_id = "E999999".into();
        assert!(validate_report_sources(&report).is_err());
        report.findings[0].evidence[0].evidence_id = "E999x".into();
        assert!(validate_report_sources(&report).is_err());
    }
}
