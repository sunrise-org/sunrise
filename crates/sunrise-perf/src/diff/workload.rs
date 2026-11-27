use serde::{Deserialize, Serialize};
use sunrise_perf_format::v1::Side;

use crate::health::{live, mean};
use crate::view::View;
use crate::wire::num;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workload {
    pub players: Option<f64>,
    pub chunks: Option<f64>,
    pub entities: Option<f64>,
    pub render_distance: Option<u32>,
    pub resolution: Option<[u32; 2]>,
    pub ms_per_tick_per_1000_chunks: Option<f64>,
    pub ms_per_tick_per_1000_entities: Option<f64>,
}

impl Workload {
    pub(super) fn read(view: &View<'_>, side: Side, thread: Option<u32>) -> Self {
        let capture = view.capture;
        let health = live(capture, side);
        let start = capture.start.iter().find(|s| s.side() == side);
        let snapshot = capture
            .world
            .iter()
            .filter(|w| w.side() == side && !w.dims.is_empty())
            .max_by(|a, b| a.t.total_cmp(&b.t));
        let average = |pick: fn(&sunrise_perf_format::v1::HealthSample) -> u32| {
            mean(health.iter().map(|h| f64::from(pick(h))))
        };
        let players = average(|h| h.players).or_else(|| {
            start
                .filter(|_| side == Side::Server)
                .map(|s| f64::from(s.players))
        });
        let chunks = average(|h| h.chunks)
            .or_else(|| snapshot.map(|w| w.dims.iter().map(|d| f64::from(d.chunks)).sum()));
        let entities = average(|h| h.entities).or_else(|| {
            snapshot.map(|w| {
                w.dims
                    .iter()
                    .flat_map(|d| &d.entities)
                    .map(|e| f64::from(e.count))
                    .sum()
            })
        });
        let mut workload = Self {
            players,
            chunks,
            entities,
            ..Self::default()
        };
        match side {
            Side::Client => {
                let report = capture
                    .bench
                    .iter()
                    .filter(|b| b.side() == side)
                    .find_map(|b| sunrise_bench::Report::parse(&b.report_json).ok());
                workload.render_distance = start
                    .and_then(|s| s.client_settings.as_ref())
                    .and_then(|s| s.view_distance)
                    .or_else(|| report.as_ref().and_then(|r| r.conditions.render_distance));
                workload.resolution = report.and_then(|r| r.conditions.window);
            }
            Side::Server => {
                let mspt =
                    mean(health.iter().filter_map(|h| h.mspt_avg.map(f64::from))).or_else(|| {
                        let s = view.summary(side, thread?)?;
                        (s.count > 0 && s.wall_ms_total > 0.0)
                            .then(|| s.wall_ms_total / s.count as f64)
                    });
                let per =
                    |count: Option<f64>| Some(num(mspt? * 1000.0 / count.filter(|n| *n > 0.0)?));
                workload.ms_per_tick_per_1000_chunks = per(chunks);
                workload.ms_per_tick_per_1000_entities = per(entities);
            }
            _ => {}
        }
        workload
    }

    pub(super) fn text(&self, role: &str, file: &str, side: &str) -> String {
        let count = |n: Option<f64>| n.map_or_else(|| "unknown".into(), |n| format!("{n:.1}"));
        let mut line = format!(
            "workload {side} {role} {file}: {} players · {} loaded chunks · {} entities",
            count(self.players),
            count(self.chunks),
            count(self.entities)
        );
        if let Some(distance) = self.render_distance {
            line.push_str(&format!(" · render distance {distance} chunks"));
        }
        if let Some([width, height]) = self.resolution {
            line.push_str(&format!(" · {width}×{height} window"));
        }
        for (value, label) in [
            (self.ms_per_tick_per_1000_chunks, "chunks"),
            (self.ms_per_tick_per_1000_entities, "entities"),
        ] {
            if let Some(ms) = value {
                line.push_str(&format!(" · {ms:.2} ms/tick per 1000 {label}"));
            }
        }
        line
    }
}

pub(super) fn warnings(a: Workload, b: Workload) -> Vec<String> {
    let pixels = |size: Option<[u32; 2]>| size.map(|[w, h]| f64::from(w) * f64::from(h));
    let changed: Vec<_> = [
        ("players", a.players, b.players),
        ("loaded chunks", a.chunks, b.chunks),
        ("entities", a.entities, b.entities),
        (
            "render distance",
            a.render_distance.map(f64::from),
            b.render_distance.map(f64::from),
        ),
        (
            "window pixel count",
            pixels(a.resolution),
            pixels(b.resolution),
        ),
    ]
    .into_iter()
    .filter_map(|(label, a, b)| {
        let (a, b) = (a?, b?);
        ((b - a).abs() > a * 0.1).then(|| format!("{label} {a:.1} → {b:.1}"))
    })
    .collect();
    if changed.is_empty() {
        Vec::new()
    } else {
        vec![format!(
            "workloads differ by more than 10% from A: {}",
            changed.join(" · ")
        )]
    }
}
