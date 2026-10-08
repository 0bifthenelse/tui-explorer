//! Media player chrome: the windowed now-playing / video modal and the
//! fullscreen video strip.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::state::{AppState, Mode};
use crate::ui::format::truncate;
use crate::ui::hit::{HitMap, HitTarget};
use crate::ui::modals::centered_rect;
use crate::ui::palette::*;
use crate::ui::widgets::{Button, ButtonState, button_row, draw_button, rail_geometry};
use crate::ui::{
    accent_border_style, dir_style, error_style, muted_style, overlay_block, preview_meta_style,
    surface_fill,
};

/// Seconds shown as "elapsed" while a rail drag scrubs: the drag position
/// when one is in flight, else the reported position (spec section 1).
fn media_display_position(media: &crate::app::state::MediaState) -> f64 {
    if media.slider_drag_active {
        media.slider_drag_pos.unwrap_or(media.position)
    } else {
        media.position
    }
}

/// Button visual state per spec section 7: Hovered comes from
/// `state.hover.control`; Active marks the transport toggle while the
/// session is live (Playing or Paused, mirroring the pre-existing check)
/// and FULL while fullscreen is engaged. Hover wins over Active so pointer
/// feedback is always visible (`draw_button` treats states as exclusive).
fn media_button_state(
    state: &AppState,
    media: &crate::app::state::MediaState,
    target: HitTarget,
) -> ButtonState {
    use crate::media::{MediaKind, MediaPhase};

    if state.hover.control == Some(target) {
        return ButtonState::Hovered;
    }
    let active = match target {
        HitTarget::MediaTogglePause => {
            matches!(media.phase, MediaPhase::Playing | MediaPhase::Paused)
        }
        HitTarget::MediaFullscreen => media.kind == MediaKind::Video && media.fullscreen,
        _ => false,
    };
    if active {
        ButtonState::Active
    } else {
        ButtonState::Idle
    }
}

/// Draws the seek rail inside `rect` (spec section 1 row 2 draw order):
/// played track ACCENT+BOLD `━`, remainder BORDER_SUBTLE `─`, hover tick
/// `│` in ACCENT_HOVER plus an optional floating timestamp beside the
/// tick, then the ACCENT_HOVER `●` thumb drawn last so it wins where the
/// tick and thumb coincide. Registers `HitTarget::MediaSeekRail`
/// unconditionally — including unknown duration; the reducer gates
/// unknown-duration gestures, not the renderer.
fn draw_seek_rail(
    frame: &mut Frame,
    rect: Rect,
    media: &crate::app::state::MediaState,
    hits: &mut HitMap,
    floating_label: bool,
) {
    let display_position = media_display_position(media);
    let mut geom = rail_geometry(rect, display_position, media.duration);
    if !media.slider_drag_active
        && let Some(hover_secs) = media.slider_hover
    {
        // Same geometry for preview and commit: the tick lands exactly
        // where a click there would seek (spec section 1 row 2).
        geom.hover_x = Some(rail_geometry(rect, hover_secs, media.duration).thumb_x);
    }

    let width = rect.width as usize;
    let played = geom.thumb_x.saturating_sub(rect.x) as usize;
    let buffer = frame.buffer_mut();
    if played > 0 {
        buffer.set_stringn(
            rect.x,
            rect.y,
            "\u{2501}".repeat(played),
            played,
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        );
    }
    let rest = width.saturating_sub(played);
    if rest > 0 {
        buffer.set_stringn(
            rect.x + played as u16,
            rect.y,
            "\u{2500}".repeat(rest),
            rest,
            Style::default().fg(BORDER_SUBTLE),
        );
    }
    if let Some(hover_x) = geom.hover_x
        && hover_x >= rect.x
        && hover_x < rect.x + rect.width
    {
        buffer.set_stringn(
            hover_x,
            rect.y,
            "\u{2502}",
            1,
            Style::default().fg(ACCENT_HOVER),
        );
        if floating_label && let Some(hover_secs) = media.slider_hover {
            // Spec section 1: label at hover_x+2 when six columns fit to
            // the right of the tick, else at hover_x-7 when they fit to
            // the left, else omitted.
            let right_edge = rect.x + rect.width;
            let label_x = if hover_x + 8 <= right_edge {
                Some(hover_x + 2)
            } else if hover_x >= rect.x + 7 {
                Some(hover_x - 7)
            } else {
                None
            };
            if let Some(label_x) = label_x {
                let stamp =
                    format_time_duration(std::time::Duration::from_secs_f64(hover_secs.max(0.0)));
                buffer.set_stringn(
                    label_x,
                    rect.y,
                    &stamp,
                    6,
                    Style::default().fg(ACCENT_HOVER),
                );
            }
        }
    }
    // Thumb last: always wins over the hover tick when they coincide.
    if geom.thumb_x >= rect.x && geom.thumb_x < rect.x + rect.width {
        buffer.set_stringn(
            geom.thumb_x,
            rect.y,
            "\u{25CF}",
            1,
            Style::default().fg(ACCENT_HOVER),
        );
    }
    hits.push(rect, HitTarget::MediaSeekRail);
}

pub(crate) fn render_media_modal(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    media: &crate::app::state::MediaState,
) {
    use crate::media::{MediaKind, MediaPhase};

    state.hit_map.push(area, HitTarget::Blocker);
    if media.kind == MediaKind::Video && media.fullscreen && media.backend.in_terminal() {
        render_media_fullscreen(frame, area, state, media);
        return;
    }
    let full = area.width >= 60 && area.height >= 16;
    let modal_height = if full { 22 } else { 12 }.min(area.height);
    let rect = centered_rect(area, area.width.min(96), modal_height);
    frame.render_widget(Clear, rect);
    let title = match media.kind {
        MediaKind::Audio => "NOW PLAYING",
        MediaKind::Video => "VIDEO",
    };
    let block = overlay_block(title, accent_border_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width < 8 || inner.height < 7 {
        return;
    }

    // Control tiers key off inner.width (spec section 4): Wide >= 60 one
    // row, Split 40..60 two rows, Pruned < 40 drops NEXT/V-/V+ (video
    // keeps FULL). Clipped windows below the compact two-row budget
    // degrade to one control row; every other size matches the spec table.
    let controls_rows: u16 = if (40..60).contains(&inner.width) && inner.height >= 10 {
        2
    } else {
        1
    };
    let controls_height = controls_rows * 3;
    let controls_top = inner.y + inner.height - controls_height;
    // Row budget (spec section 1): header rows 0-2 fixed; Full mode adds a
    // spacer after the header and before the controls, Compact mode has
    // neither.
    let (surface_top, surface_height) = if full {
        (
            inner.y + 4,
            inner.height.saturating_sub(4 + controls_height + 1),
        )
    } else {
        (
            inner.y + 3,
            inner.height.saturating_sub(3 + controls_height),
        )
    };

    // Row 0: filename (left) + phase chip flush right (spec section 1).
    let phase_text = if media.error.is_some() {
        "ERROR".to_string()
    } else {
        format!("{:?}", media.phase).to_ascii_uppercase()
    };
    let chip = format!(" {phase_text} ");
    let chip_width = (chip.chars().count() as u16).min(inner.width);
    let chip_style = if media.error.is_some() {
        Style::default()
            .fg(DANGER)
            .bg(ROOT_INK)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ACCENT).bg(ROOT_INK)
    };
    frame.buffer_mut().set_stringn(
        inner.x + inner.width - chip_width,
        inner.y,
        &chip,
        chip_width as usize,
        chip_style,
    );
    let filename = media
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| media.path.display().to_string());
    let name_width = inner.width.saturating_sub(chip_width + 1);
    if name_width > 0 {
        // Tagged tracks show "Title  Artist · Album · Year".
        let mut spans = Vec::new();
        match media.tags.as_ref().filter(|t| !t.is_empty()) {
            Some(tags) => {
                let title = tags.title.clone().unwrap_or_else(|| filename.clone());
                spans.push(Span::styled(
                    truncate(&title, name_width as usize),
                    dir_style(),
                ));
                let byline = tags.byline();
                let used = crate::ui::format::display_width(&title) + 2;
                if !byline.is_empty() && used + 4 < name_width as usize {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        truncate(&byline, name_width as usize - used),
                        Style::default().fg(ACCENT_SOFT),
                    ));
                }
            }
            None => spans.push(Span::styled(
                truncate(&filename, name_width as usize),
                dir_style(),
            )),
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(inner.x, inner.y, name_width, 1),
        );
    }

    // Row 1: time row, replaced wholesale by the error message on error.
    // While a drag scrubs, elapsed follows the drag position; remaining
    // stays anchored to the committed position (spec section 1).
    if let Some(error) = media.error.as_deref() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate(&format!("[!] {error}"), inner.width as usize),
                error_style(),
            ))),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    } else {
        let display_position = media_display_position(media);
        let elapsed_text = format_time_duration(std::time::Duration::from_secs_f64(
            display_position.max(0.0),
        ));
        let duration_text = media
            .duration
            .map(|seconds| {
                format_time_duration(std::time::Duration::from_secs_f64(seconds.max(0.0)))
            })
            .unwrap_or_else(|| "--:--".to_string());
        let remaining_text = media
            .duration
            .map(|seconds| {
                format_time_duration(std::time::Duration::from_secs_f64(
                    (seconds - media.position).max(0.0),
                ))
            })
            .unwrap_or_else(|| "--:--".to_string());
        // Volume is the backend-reported value (sink or observed mpv
        // property), never a UI-only guess.
        let time = format!(
            "{elapsed_text} / {duration_text} / -{remaining_text} | VOL {}%",
            media.volume
        );
        let chips = status_chips(media);
        let mut spans = vec![Span::styled(time, preview_meta_style())];
        for chip in chips {
            spans.push(Span::styled(
                format!(" {} ", crate::ui::glyphs::g().dot),
                Style::default().fg(BORDER_STRONG),
            ));
            spans.push(Span::styled(chip, Style::default().fg(ACCENT_SOFT)));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    }

    // Row 2: the seek rail, drawn through the shared widgets geometry.
    draw_seek_rail(
        frame,
        Rect::new(inner.x, inner.y + 2, inner.width, 1),
        media,
        &mut state.hit_map,
        true,
    );

    let surface_rect = Rect::new(inner.x, surface_top, inner.width, surface_height);
    let surface = crate::app::state::MediaSurface {
        rect: surface_rect,
        terminal_cells: (area.width, area.height),
        cell_pixels: state.picker.font_size(),
    };
    if let Mode::Media(current) = &mut state.mode
        && current.session == media.session
    {
        current.surface = Some(surface);
    }

    // Control strip (spec sections 4 and 7): bordered buttons sized
    // label+2, laid out through `button_row`, hit registration handled by
    // `draw_button`.
    let toggle_label = if matches!(media.phase, MediaPhase::Playing) {
        "PAUSE"
    } else {
        "PLAY"
    };
    let video = media.kind == MediaKind::Video;
    let control_rows: Vec<Vec<(&str, u16, HitTarget)>> = if controls_rows == 2 {
        // Split tier: row 1 transports and exits, row 2 conveniences.
        let row2 = if video {
            vec![
                ("NEXT", 6u16, HitTarget::MediaNext),
                ("V-", 4, HitTarget::MediaVolumeDown),
                ("V+", 4, HitTarget::MediaVolumeUp),
                ("FULL", 6, HitTarget::MediaFullscreen),
            ]
        } else {
            vec![
                ("NEXT", 6u16, HitTarget::MediaNext),
                ("V-", 4, HitTarget::MediaVolumeDown),
                ("V+", 4, HitTarget::MediaVolumeUp),
            ]
        };
        vec![
            vec![
                ("-15", 5u16, HitTarget::MediaSeekBack),
                (toggle_label, 7, HitTarget::MediaTogglePause),
                ("+15", 5, HitTarget::MediaSeekForward),
                ("STOP", 6, HitTarget::MediaStop),
                ("X", 3, HitTarget::MediaClose),
            ],
            row2,
        ]
    } else {
        let mut row = vec![
            ("-15", 5u16, HitTarget::MediaSeekBack),
            (toggle_label, 7, HitTarget::MediaTogglePause),
            ("+15", 5, HitTarget::MediaSeekForward),
        ];
        if inner.width >= 60 {
            row.extend([
                ("NEXT", 6u16, HitTarget::MediaNext),
                ("V-", 4, HitTarget::MediaVolumeDown),
                ("V+", 4, HitTarget::MediaVolumeUp),
            ]);
        }
        if video {
            // FULL survives pruning: the escape hatch for cramped terms.
            row.push(("FULL", 6, HitTarget::MediaFullscreen));
        }
        row.push(("STOP", 6, HitTarget::MediaStop));
        row.push(("X", 3, HitTarget::MediaClose));
        vec![row]
    };
    for (index, specs) in control_rows.iter().enumerate() {
        let y = controls_top + index as u16 * 3;
        let layout: Vec<(&str, u16)> = specs
            .iter()
            .map(|(label, width, _)| (*label, *width))
            .collect();
        // `button_row` lays out horizontally at height 1; bordered buttons
        // are always 3 rows tall (spec section 4).
        for (btn_rect, (label, _, target)) in button_row(inner.x, y, inner.width, &layout)
            .iter()
            .zip(specs)
        {
            let button = Button::new(
                Rect {
                    height: 3,
                    ..*btn_rect
                },
                *label,
                *target,
            )
            .with_state(media_button_state(state, media, *target));
            draw_button(frame, &button, &mut state.hit_map);
        }
    }

    match media.kind {
        MediaKind::Audio => {
            if media.error.is_none() && surface_rect.height > 0 {
                // Queue panel on the right when there is room for it.
                let mut bars_rect = surface_rect;
                if surface_rect.width >= 64 && media.playlist.len() > 1 {
                    let queue_w = (surface_rect.width * 2 / 5).clamp(24, 40);
                    let queue_rect = Rect::new(
                        surface_rect.right() - queue_w,
                        surface_rect.y,
                        queue_w,
                        surface_rect.height,
                    );
                    bars_rect.width -= queue_w + 2;
                    render_queue(frame, queue_rect, state, media);
                }
                // Cover art on the left when the track embeds one.
                if bars_rect.width >= 48
                    && state
                        .cover
                        .as_ref()
                        .is_some_and(|c| c.session == media.session)
                {
                    let h = bars_rect.height.min(12);
                    let w = (h * 2).min(bars_rect.width / 3);
                    let cover_rect = Rect::new(bars_rect.x, bars_rect.y, w, h);
                    if let Some(cover) = state.cover.as_mut() {
                        frame.render_stateful_widget(
                            ratatui_image::StatefulImage::new(),
                            cover_rect,
                            cover.image.as_mut(),
                        );
                    }
                    bars_rect.x += w + 2;
                    bars_rect.width = bars_rect.width.saturating_sub(w + 2);
                }
                render_spectrum(frame, bars_rect, state, media);
            } else if media.error.is_none() {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        if matches!(media.phase, MediaPhase::Preparing | MediaPhase::Starting) {
                            "spectrum starting"
                        } else {
                            "spectrum unavailable at this size"
                        },
                        muted_style(),
                    ))),
                    surface_rect,
                );
            }
        }
        // Video owns the surface: while frames are live (Playing/Paused)
        // nothing is drawn there so mpv's kitty output survives the
        // diff-based redraws. Only startup shows placeholder chrome
        // (spec section 2). Controls live outside `surface_rect` by
        // construction: header/rail/spacers/controls never intersect it.
        MediaKind::Video => {
            if media.error.is_none()
                && surface_rect.height > 0
                && media.backend == crate::media::VideoBackend::Window
            {
                // The video plays in mpv's own window; the modal is the remote.
                let lines = vec![
                    Line::from(Span::styled(
                        format!("{}  playing in its own window", crate::ui::glyphs::g().play),
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        "f fullscreen · c subtitles · Space pause · q stop",
                        muted_style(),
                    )),
                ];
                let mid = surface_rect.y + surface_rect.height.saturating_sub(2) / 2;
                frame.render_widget(
                    Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
                    Rect::new(
                        surface_rect.x,
                        mid,
                        surface_rect.width,
                        2.min(surface_rect.height),
                    ),
                );
            } else if media.error.is_none()
                && surface_rect.height > 0
                && matches!(media.phase, MediaPhase::Preparing | MediaPhase::Starting)
            {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled("loading video", muted_style())))
                        .alignment(ratatui::layout::Alignment::Center),
                    surface_rect,
                );
            }
        }
    }
}

/// Short state chips after the time row: queue position, shuffle, repeat,
/// speed, mute and subtitle state.
fn status_chips(media: &crate::app::state::MediaState) -> Vec<String> {
    use crate::app::state::Repeat;
    use crate::ui::glyphs::g;
    let mut chips = Vec::new();
    if media.playlist.len() > 1 {
        chips.push(format!(
            "{}/{}",
            media.playlist_pos + 1,
            media.playlist.len()
        ));
    }
    if media.shuffle {
        chips.push(format!("{} shuffle", g().shuffle));
    }
    match media.repeat {
        Repeat::Off => {}
        Repeat::All => chips.push(format!("{} all", g().repeat)),
        Repeat::One => chips.push(format!("{} one", g().repeat)),
    }
    if (media.speed - 1.0).abs() > f64::EPSILON {
        chips.push(format!("{:.2}×", media.speed));
    }
    if media.muted {
        chips.push(format!("{} muted", g().mute));
    }
    if media.kind == crate::media::MediaKind::Video {
        if media.backend != crate::media::VideoBackend::Kitty {
            chips.push(format!("via {}", media.backend.label()));
        }
        if media.subs_off {
            chips.push("subs off".to_string());
        } else if let Some(sub) = &media.sub_file {
            let name = sub
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            chips.push(format!("{} {}", g().subtitles, truncate(&name, 28)));
        }
        if media.sub_delay.abs() > f64::EPSILON {
            chips.push(format!("delay {:+.1}s", media.sub_delay));
        }
    }
    chips
}

/// Smooth gradient spectrum: one bar per band with sub-cell heights,
/// each band eased so the motion flows between analyzer snapshots.
fn render_spectrum(
    frame: &mut Frame,
    rect: Rect,
    state: &mut AppState,
    media: &crate::app::state::MediaState,
) {
    use crate::ui::anim::AnimKey;
    use crate::ui::glyphs::g;
    use crate::ui::theme::{accent_gradient, mix};
    let bands = media.spectrum.len().max(1) as u16;
    if rect.width < bands || rect.height == 0 {
        return;
    }
    let col_w = (rect.width / bands).max(1);
    let bar_w = if col_w >= 3 { col_w - 1 } else { col_w };
    let height = rect.height.min(12);
    let top = rect.y + rect.height - height;
    let left = rect.x + (rect.width - col_w * bands) / 2;
    let playing = media.phase == crate::media::MediaPhase::Playing;
    for (index, level) in media.spectrum.iter().enumerate() {
        let target = if playing { level.clamp(0.0, 1.0) } else { 0.0 };
        let shown = state.anim.track(
            AnimKey::Indexed("band", index as u32),
            target,
            std::time::Duration::from_millis(110),
        );
        let eighths = (shown * f32::from(height) * 8.0).round() as u16;
        let x = left + index as u16 * col_w;
        let hue = accent_gradient(index as f32 / f32::from(bands - 1).max(1.0));
        for row in 0..height {
            let fill_level = eighths.saturating_sub(row * 8).min(8) as usize;
            let y = top + height - 1 - row;
            let (sym, color) = if fill_level == 0 {
                // A faint floor keeps the analyzer's shape while quiet.
                if row == 0 {
                    (g().eighths_v[1], BORDER_SUBTLE)
                } else {
                    continue;
                }
            } else {
                let lift = f32::from(row) / f32::from(height.max(1));
                (
                    g().eighths_v[fill_level],
                    mix(hue, ACCENT_AMBER, lift * 0.6),
                )
            };
            for dx in 0..bar_w {
                frame
                    .buffer_mut()
                    .set_stringn(x + dx, y, sym, 1, Style::default().fg(color));
            }
        }
    }
}

/// "Up next" queue: the current track highlighted, clickable rows.
fn render_queue(
    frame: &mut Frame,
    rect: Rect,
    state: &mut AppState,
    media: &crate::app::state::MediaState,
) {
    use crate::ui::glyphs::g;
    use crate::ui::{control_glow, hover_bg, put};
    let buf = frame.buffer_mut();
    put(
        buf,
        rect.x,
        rect.y,
        "UP NEXT",
        rect.width,
        Style::default().fg(TEXT_MUTED).add_modifier(Modifier::BOLD),
    );
    let rows = rect.height.saturating_sub(1) as usize;
    let start = media.playlist_pos.saturating_sub(1);
    for (row, (idx, path)) in media
        .playlist
        .iter()
        .enumerate()
        .skip(start)
        .take(rows)
        .enumerate()
    {
        let y = rect.y + 1 + row as u16;
        let target = HitTarget::QueueRow(idx);
        let glow = control_glow(state, target);
        let current = idx == media.playlist_pos;
        let bg = if current {
            ACCENT
        } else {
            hover_bg(SURFACE_3, glow)
        };
        let fg = if current {
            INK_ON_ACCENT
        } else if idx < media.playlist_pos {
            TEXT_MUTED
        } else {
            TEXT_SECONDARY
        };
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let marker = if current { g().play } else { " " };
        let text = format!(" {marker} {:>2} {}", idx + 1, name);
        let buf = frame.buffer_mut();
        crate::ui::fill(buf, Rect::new(rect.x, y, rect.width, 1), bg);
        put(
            buf,
            rect.x,
            y,
            &truncate(&text, rect.width as usize),
            rect.width,
            Style::default().fg(fg).bg(bg),
        );
        state
            .hit_map
            .push(Rect::new(rect.x, y, rect.width, 1), target);
    }
}

/// Fullscreen video chrome (spec section 3): the whole area clears for
/// mpv and a 2-row bottom strip carries time+rail (row 1) and filename
/// plus flat bracket-text controls (row 2). Bordered buttons need 3 rows,
/// which cannot fit the hard 2-row budget, so this is the one place
/// bracket controls remain (spec section 0).
fn render_media_fullscreen(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    media: &crate::app::state::MediaState,
) {
    use crate::media::MediaPhase;

    frame.render_widget(Clear, area);
    // Text output gets the whole terminal: while it plays the TUI stops
    // drawing (two writers would interleave escape sequences) and mpv's
    // OSD carries the feedback; paused, the strip below returns.
    let video_rect = if media.backend == crate::media::VideoBackend::Tct {
        area
    } else {
        Rect::new(area.x, area.y, area.width, area.height.saturating_sub(2))
    };
    // Surface geometry flows through the awaiting_surface_ready restart
    // cycle: these two rects are what the stop->Preparing->SurfaceReady
    // restart resolves into (spec section 3).
    let surface = crate::app::state::MediaSurface {
        rect: video_rect,
        terminal_cells: (area.width, area.height),
        cell_pixels: state.picker.font_size(),
    };
    if let Mode::Media(current) = &mut state.mode
        && current.session == media.session
    {
        current.surface = Some(surface);
    }
    if area.height < 2 {
        return;
    }
    let strip_y = area.y + area.height - 2;
    surface_fill(frame, Rect::new(area.x, strip_y, area.width, 2), SURFACE_2);

    // Strip row 1: elapsed | rail | -remaining, seven columns reserved at
    // each end; on error the whole row becomes the error text.
    if let Some(error) = media.error.as_deref() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate(&format!("[!] {error}"), area.width as usize),
                error_style(),
            ))),
            Rect::new(area.x, strip_y, area.width, 1),
        );
    } else {
        let elapsed_text = format_time_duration(std::time::Duration::from_secs_f64(
            media_display_position(media).max(0.0),
        ));
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                elapsed_text,
                Style::default().fg(TEXT_PRIMARY),
            ))),
            Rect::new(area.x, strip_y, 6.min(area.width), 1),
        );
        let remaining_text = match media.duration {
            Some(seconds) => format!(
                "-{}",
                format_time_duration(std::time::Duration::from_secs_f64(
                    (seconds - media.position).max(0.0)
                ))
            ),
            None => "--:--".to_string(),
        };
        let remaining_width = 7u16.min(area.width);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                remaining_text,
                Style::default().fg(TEXT_PRIMARY),
            ))),
            Rect::new(
                area.x + area.width - remaining_width,
                strip_y,
                remaining_width,
                1,
            ),
        );
        draw_seek_rail(
            frame,
            Rect::new(area.x + 7, strip_y, area.width.saturating_sub(14), 1),
            media,
            &mut state.hit_map,
            // No floating timestamp here: no spare width; the bare hover
            // tick still renders (spec section 3).
            false,
        );
    }

    // Strip row 2: filename (at most 24 cols) + 1-col gap + flat bracket
    // controls using the overflow-guard loop pattern; colors mirror the
    // ButtonState matrix fg-only (spec section 3).
    let filename = media
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| media.path.display().to_string());
    frame.buffer_mut().set_stringn(
        area.x,
        strip_y + 1,
        truncate(&filename, 24),
        (area.width as usize).min(24),
        dir_style(),
    );
    let toggle_label = if matches!(media.phase, MediaPhase::Playing) {
        "[PAUSE]"
    } else {
        "[PLAY]"
    };
    let controls: [(&str, u16, HitTarget); 9] = [
        ("[-15]", 5, HitTarget::MediaSeekBack),
        (toggle_label, 7, HitTarget::MediaTogglePause),
        ("[+15]", 5, HitTarget::MediaSeekForward),
        ("[NEXT]", 6, HitTarget::MediaNext),
        ("[V-]", 4, HitTarget::MediaVolumeDown),
        ("[V+]", 4, HitTarget::MediaVolumeUp),
        ("[FULL]", 6, HitTarget::MediaFullscreen),
        ("[STOP]", 6, HitTarget::MediaStop),
        ("[X]", 3, HitTarget::MediaClose),
    ];
    let mut control_x = area.x + 25; // 24-col filename + 1-col gap
    for (label, width, target) in controls {
        if control_x + width > area.x + area.width {
            break;
        }
        let style = if state.hover.control == Some(target) {
            Style::default().fg(ACCENT_HOVER)
        } else if target == HitTarget::MediaFullscreen {
            // This branch IS the fullscreen-on state indicator.
            accent_border_style()
        } else if target == HitTarget::MediaTogglePause
            && matches!(media.phase, MediaPhase::Playing)
        {
            accent_border_style()
        } else {
            Style::default().fg(TEXT_SECONDARY)
        };
        frame
            .buffer_mut()
            .set_stringn(control_x, strip_y + 1, label, width as usize, style);
        state
            .hit_map
            .push(Rect::new(control_x, strip_y + 1, width, 1), target);
        control_x += width + 1;
    }
}

fn format_time_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// Render-contract tests for the designer-spec media chrome (spec §1-§5).
#[cfg(test)]
mod media_render_tests {
    use super::*;
    use crate::ui::render;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    use crate::app::state::MediaState;
    use crate::media::{MediaKind, MediaPhase};

    fn row_text(buffer: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        line
    }

    fn rendered(state: &mut AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
        state.width = width;
        state.height = height;
        let backend = TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, state)).expect("draw");
        terminal.backend().buffer().clone()
    }

    fn playing_audio(duration: Option<f64>) -> (AppState, MediaState) {
        let state = AppState::new(PathBuf::from("/"), PathBuf::from("/tmp"));
        let mut media = MediaState::preparing(7, PathBuf::from("/tmp/song.mp3"), MediaKind::Audio);
        media.phase = MediaPhase::Playing;
        media.position = 30.0;
        media.duration = duration;
        (state, media)
    }

    #[test]
    fn audio_modal_shows_chip_time_rail_and_button_labels() {
        use crate::app::state::Mode;
        let (mut state, media) = playing_audio(Some(90.0));
        state.mode = Mode::Media(Box::new(media));
        let buffer = rendered(&mut state, 100, 30);
        let text: String = (0..30)
            .map(|y| format!("{}|", row_text(&buffer, y, 100)))
            .collect();
        assert!(text.contains("PLAYING"), "phase chip: {text}");
        assert!(
            text.contains("00:30 / 01:30 / -01:00 | VOL 100%"),
            "time row format per spec section 1: {text}"
        );
        assert!(text.contains('\u{25CF}'), "rail thumb drawn");
        assert!(text.contains('\u{2501}'), "played rail segment drawn");
        for label in ["-15", "PAUSE", "+15", "NEXT", "V-", "V+", "STOP", "X"] {
            assert!(text.contains(label), "button label {label}: {text}");
        }
        // Wide tier keeps the video-only FULL button out of audio modals.
        assert!(!text.contains("FULL "), "no FULL button on audio");
    }

    #[test]
    fn rail_hit_registered_even_when_duration_is_unknown() {
        use crate::app::state::Mode;
        let (mut state, mut media) = playing_audio(None);
        media.duration = None;
        state.mode = Mode::Media(Box::new(media));
        let _buffer = rendered(&mut state, 100, 30);
        let rail = state.hit_map.rect_for(HitTarget::MediaSeekRail);
        assert!(
            rail.is_some_and(|rect| rect.width > 0),
            "MediaSeekRail must be registered without a duration"
        );
    }

    #[test]
    fn split_tier_lays_two_control_rows_and_pruned_tier_drops_conveniences() {
        use crate::app::state::Mode;
        // Split tier: 40 <= inner.width < 60.
        let (mut state, media) = playing_audio(Some(90.0));
        state.mode = Mode::Media(Box::new(media));
        rendered(&mut state, 50, 24);
        assert!(
            state.hit_map.rect_for(HitTarget::MediaNext).is_some(),
            "split tier registers NEXT"
        );

        // Pruned tier: inner.width < 40 drops NEXT/V-/V+, keeps STOP/X.
        let (mut state, media) = playing_audio(Some(90.0));
        state.mode = Mode::Media(Box::new(media));
        let _buffer = rendered(&mut state, 36, 20);
        assert!(
            state.hit_map.rect_for(HitTarget::MediaNext).is_none(),
            "pruned tier drops NEXT"
        );
        assert!(
            state.hit_map.rect_for(HitTarget::MediaVolumeDown).is_none(),
            "pruned tier drops V-"
        );
        assert!(state.hit_map.rect_for(HitTarget::MediaStop).is_some());
        assert!(state.hit_map.rect_for(HitTarget::MediaClose).is_some());
    }

    #[test]
    fn fullscreen_video_reserves_two_row_strip_and_clears_the_rest() {
        use crate::app::state::Mode;
        let (mut state, mut media) = playing_audio(Some(90.0));
        media.kind = MediaKind::Video;
        media.fullscreen = true;
        state.mode = Mode::Media(Box::new(media));
        let width = 90u16;
        let height = 26u16;
        let buffer = rendered(&mut state, width, height);
        // Surface gets everything above the strip.
        if let Mode::Media(current) = &state.mode {
            let surface = current.surface.expect("surface registered");
            assert_eq!(surface.rect.height, height - 2, "video rect is area-2");
            assert_eq!(surface.rect.y, 0);
        } else {
            panic!("media mode expected");
        }
        // Strip rows hold transport and flat bracket controls.
        let strip_text = format!(
            "{}\n{}",
            row_text(&buffer, height - 2, width),
            row_text(&buffer, height - 1, width)
        );
        assert!(
            strip_text.contains("[PAUSE]"),
            "flat controls: {strip_text}"
        );
        assert!(strip_text.contains("[X]"), "close control: {strip_text}");
        assert!(
            strip_text.contains("-01:00"),
            "remaining time: {strip_text}"
        );
        // The cleared region above the strip must not leak modal borders.
        assert_ne!(
            buffer[(0, 0)].symbol(),
            "+",
            "fullscreen clears windowed chrome"
        );
    }

    #[test]
    fn context_menu_background_titles_cwd_and_disabled_paste_has_no_hit() {
        use crate::app::state::{
            ClipboardState, ContextItem, ContextMenuState, ContextTarget, MenuItem, Mode,
        };
        let mut state = AppState::new(PathBuf::from("/home/u/docs"), PathBuf::from("/"));
        state.mode = Mode::ContextMenu(Box::new(ContextMenuState {
            target: ContextTarget::Background,
            items: vec![MenuItem {
                action: ContextItem::Paste,
                enabled: false,
            }],
            selected: 0,
            x: 2,
            y: 2,
        }));
        state.clipboard = ClipboardState::default();
        let buffer = rendered(&mut state, 60, 20);
        // Title row (menu top border) carries the cwd basename.
        let top = row_text(&buffer, 2, 60);
        assert!(
            top.contains("docs"),
            "background title is cwd basename: {top}"
        );
        // Disabled Paste renders but registers no hit.
        assert!(
            !state
                .hit_map
                .regions
                .iter()
                .any(|(_, t)| matches!(t, HitTarget::ContextItem(_))),
            "disabled paste registers no hit"
        );
    }
}
