// v1.0.2.0 Windows - Make appearance cards report actual selection changes.
// v2.22.46 - Shared Windows navigation, surfaces and responsive layout.

fn desktop_palette(preference: ThemePreference) -> Palette {
    let dark = matches!(preference, ThemePreference::Dark | ThemePreference::Oled);
    let oled = preference == ThemePreference::Oled;
    let rgb = |hex: u32| egui::Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
    if dark {
        Palette {
            is_dark: true,
            bg: rgb(if oled { 0x000000 } else { 0x141820 }),
            nav: rgb(if oled { 0x080B10 } else { 0x191F29 }),
            panel: rgb(if oled { 0x10141B } else { 0x1D2430 }),
            panel_alt: rgb(0x262F3D),
            selected: rgb(0x293B60),
            text: rgb(0xEAF0FA),
            muted: rgb(0xA5B2C6),
            line: rgb(0x344052),
            accent: rgb(0xA1BCFF),
            accent_soft: rgb(0x293B60),
            good: rgb(0x77D9B6),
            warn: rgb(0xEBC083),
            blue: rgb(0xA1BCFF),
            danger: rgb(0xFFACA9),
            danger_soft: rgb(0x482B32),
            on_accent: rgb(0x13264B),
        }
    } else {
        Palette {
            is_dark: false,
            bg: rgb(0xF4F6FA),
            nav: rgb(0xEDF1F7),
            panel: rgb(0xFFFFFF),
            panel_alt: rgb(0xF0F3F8),
            selected: rgb(0xE6EDFF),
            text: rgb(0x202D43),
            muted: rgb(0x5E6E85),
            line: rgb(0xDDE4EE),
            accent: rgb(0x315CBE),
            accent_soft: rgb(0xE6EDFF),
            good: rgb(0x18765D),
            warn: rgb(0x90600D),
            blue: rgb(0x315CBE),
            danger: rgb(0xB33340),
            danger_soft: rgb(0xFCECEF),
            on_accent: rgb(0xFFFFFF),
        }
    }
}

fn desktop_content_margin(width: f32) -> f32 {
    if width < 1000.0 {
        18.0
    } else {
        28.0
    }
}

fn desktop_nav_icon(
    painter: &egui::Painter,
    center: egui::Pos2,
    tab: AppTab,
    color: egui::Color32,
) {
    let stroke = egui::Stroke::new(1.7, color);
    let point = |x, y| center + egui::vec2(x, y);
    let line = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment([point(a.0, a.1), point(b.0, b.1)], stroke);
    };
    match tab {
        AppTab::Board => {
            painter.circle_stroke(center, 8.0, stroke);
            line((0.0, -5.0), (0.0, 0.0));
            line((0.0, 0.0), (4.0, 2.5));
        }
        AppTab::Notes => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(16.0, 18.0)),
                3.0,
                stroke,
            );
            line((-4.0, -3.0), (4.0, -3.0));
            line((-4.0, 2.0), (2.0, 2.0));
        }
        AppTab::Knowledge => {
            for x in [-8.0, 1.0] {
                painter.rect_stroke(
                    egui::Rect::from_min_size(point(x, -8.0), egui::vec2(7.0, 16.0)),
                    1.5,
                    stroke,
                );
            }
        }
        AppTab::History => {
            painter.circle_stroke(center, 8.0, stroke);
            line((0.0, -5.0), (0.0, 0.0));
            line((0.0, 0.0), (-4.0, 2.0));
            line((-10.0, -8.0), (-10.0, -3.0));
            line((-10.0, -3.0), (-5.0, -3.0));
        }
        AppTab::Finance => {
            for (x, h) in [(-7.0, 7.0), (0.0, 12.0), (7.0, 17.0)] {
                painter.rect_filled(
                    egui::Rect::from_min_size(point(x - 2.0, 9.0 - h), egui::vec2(4.0, h)),
                    1.2,
                    color,
                );
            }
        }
        AppTab::My => {
            painter.circle_stroke(point(0.0, -5.0), 4.0, stroke);
            painter.rect_stroke(
                egui::Rect::from_min_size(point(-7.0, 2.0), egui::vec2(14.0, 8.0)),
                4.0,
                stroke,
            );
        }
    }
}

fn desktop_nav_button(
    ui: &mut egui::Ui,
    tab: AppTab,
    selected: bool,
    compact: bool,
) -> egui::Response {
    let p = palette();
    let short = ui.ctx().screen_rect().height() < 620.0;
    let height = match (compact, short) {
        (true, true) => 48.0,
        (true, false) => 58.0,
        (false, true) => 38.0,
        (false, false) => 46.0,
    };
    let response = ui.add_sized(
        [ui.available_width(), height],
        egui::Button::new("")
            .fill(if selected {
                p.accent_soft
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(egui::Stroke::NONE)
            .rounding(9.0),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, selected, tab.title())
    });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(("desktop_nav", tab.title())), response.rect));
    if response.hovered() && !selected {
        ui.painter().rect_filled(response.rect, 9.0, p.panel_alt);
    }
    if selected {
        let line = egui::Rect::from_center_size(
            egui::pos2(response.rect.left() + 2.0, response.rect.center().y),
            egui::vec2(3.0, 18.0),
        );
        ui.painter().rect_filled(line, 2.0, p.accent);
    }
    let color = if selected { p.accent } else { p.muted };
    let icon_center = if compact {
        response.rect.center_top() + egui::vec2(0.0, 19.0)
    } else {
        response.rect.left_center() + egui::vec2(22.0, 0.0)
    };
    desktop_nav_icon(ui.painter(), icon_center, tab, color);
    ui.painter().text(
        if compact {
            response.rect.center_bottom() - egui::vec2(0.0, 12.0)
        } else {
            response.rect.left_center() + egui::vec2(44.0, 0.0)
        },
        if compact {
            egui::Align2::CENTER_CENTER
        } else {
            egui::Align2::LEFT_CENTER
        },
        tab.title(),
        egui::FontId::proportional(if compact { 10.5 } else { 14.5 }),
        if selected { p.accent } else { p.text },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn desktop_brand(ui: &mut egui::Ui, compact: bool) {
    let p = palette();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 10.0, p.accent);
        for (x, y, h) in [(9.0, 17.0, 10.0), (17.0, 12.0, 15.0), (25.0, 8.0, 19.0)] {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min + egui::vec2(x, y), egui::vec2(3.0, h)),
                1.5,
                p.on_accent,
            );
        }
        if !compact {
            ui.label(egui::RichText::new("十倍率").size(20.0).strong());
        }
    });
}

fn desktop_segment<T: PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    value: T,
    label: &str,
) -> egui::Response {
    let p = palette();
    let selected = *current == value;
    let mut response = ui.add(
        egui::Button::new(egui::RichText::new(label).size(13.0).color(if selected {
            p.accent
        } else {
            p.muted
        }))
        .fill(if selected {
            p.accent_soft
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE)
        .rounding(7.0)
        .min_size(egui::vec2(48.0, 32.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, selected, label)
    });
    if response.clicked() && !selected {
        *current = value;
        response.mark_changed();
    }
    response
}

fn desktop_theme_choice(
    ui: &mut egui::Ui,
    preference: ThemePreference,
    current: ThemePreference,
    label: &str,
    width: f32,
) -> egui::Response {
    let selected = preference == current;
    let p = palette();
    let sample = desktop_palette(if preference == ThemePreference::System {
        ThemePreference::Light
    } else {
        preference
    });
    let mut response = ui.add_sized(
        [width, 86.0],
        egui::Button::new("")
            .fill(p.panel)
            .stroke(egui::Stroke::new(
                if selected { 1.5 } else { 1.0 },
                if selected { p.accent } else { p.line },
            ))
            .rounding(9.0),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, selected, label)
    });
    // Callers consume this custom selector through `Response::changed`, just
    // like `desktop_segment` above. A plain Button only reports `clicked`, so
    // the old response silently discarded every unselected theme click.
    if response.clicked() && !selected {
        response.mark_changed();
    }
    let preview = egui::Rect::from_min_size(
        response.rect.min + egui::vec2(8.0, 8.0),
        egui::vec2(width - 16.0, 43.0),
    );
    let painter = ui.painter();
    painter.rect_filled(preview, 4.0, sample.bg);
    painter.rect_filled(
        egui::Rect::from_min_size(
            preview.min,
            egui::vec2(preview.width() * 0.25, preview.height()),
        ),
        3.0,
        sample.nav,
    );
    for top in [8.0, 20.0] {
        painter.rect_filled(
            egui::Rect::from_min_size(
                preview.min + egui::vec2(preview.width() * 0.35, top),
                egui::vec2(preview.width() * 0.52, 7.0),
            ),
            2.0,
            if top == 8.0 {
                sample.accent_soft
            } else {
                sample.panel
            },
        );
    }
    painter.text(
        response.rect.center_bottom() - egui::vec2(0.0, 17.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(12.0),
        if selected { p.accent } else { p.text },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
