#![allow(dead_code)]

use pony_agent_core::agent::config::{ProviderRegistryStore, ProviderSelectionResolver};
use pony_agent_core::agent::runtime::{AgentRuntime, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use std::cell::RefCell;
use std::env;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_PROMPT: &str = "请用一句话说明 Pony Agent 当前的核心职责。";

#[derive(Clone)]
struct TimedTurnEvent {
    name: String,
    observed_at_ms: u64,
    payload: TurnStreamEvent,
}

struct RecordingTurnEventSink {
    started_at: Instant,
    events: RefCell<Vec<TimedTurnEvent>>,
}

impl RecordingTurnEventSink {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            events: RefCell::new(Vec::new()),
        }
    }

    fn snapshot(&self) -> Vec<TimedTurnEvent> {
        self.events.borrow().clone()
    }
}

impl TurnEventSink for RecordingTurnEventSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events.borrow_mut().push(TimedTurnEvent {
            name: name.to_string(),
            observed_at_ms: self.started_at.elapsed().as_millis() as u64,
            payload,
        });
    }
}

#[derive(Clone)]
struct ProbeTarget {
    provider_id: String,
    provider_name: String,
    model_id: String,
    model_name: String,
    model_api_name: String,
}

#[derive(Default)]
struct ProbeCli {
    provider_id: Option<String>,
    model_id: Option<String>,
    prompt: String,
    runs: usize,
    all_selected: bool,
}

fn main() {
    let cli = parse_args(env::args().skip(1).collect());
    let registry = ProviderRegistryStore::new();
    let targets = resolve_targets(&registry, &cli);

    if targets.is_empty() {
        eprintln!("no probe targets resolved");
        std::process::exit(1);
    }

    println!("probe_runs={}", cli.runs);
    println!("probe_targets={}", targets.len());
    println!("prompt={}", cli.prompt);

    for target in targets {
        for run_index in 0..cli.runs {
            run_probe(&target, &cli.prompt, run_index + 1);
        }
    }
}

fn parse_args(args: Vec<String>) -> ProbeCli {
    let mut cli = ProbeCli {
        prompt: DEFAULT_PROMPT.to_string(),
        runs: 1,
        ..ProbeCli::default()
    };

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--provider" => {
                if let Some(value) = args.get(index + 1) {
                    cli.provider_id = Some(value.clone());
                    index += 2;
                    continue;
                }
            }
            "--model" => {
                if let Some(value) = args.get(index + 1) {
                    cli.model_id = Some(value.clone());
                    index += 2;
                    continue;
                }
            }
            "--prompt" => {
                if let Some(value) = args.get(index + 1) {
                    cli.prompt = value.clone();
                    index += 2;
                    continue;
                }
            }
            "--runs" => {
                if let Some(value) = args.get(index + 1) {
                    cli.runs = value
                        .parse::<usize>()
                        .ok()
                        .filter(|count| *count > 0)
                        .unwrap_or(1);
                    index += 2;
                    continue;
                }
            }
            "--all-selected" => {
                cli.all_selected = true;
                index += 1;
                continue;
            }
            _ => {}
        }

        index += 1;
    }

    cli
}

fn resolve_targets(registry: &ProviderRegistryStore, cli: &ProbeCli) -> Vec<ProbeTarget> {
    let view = registry.load_view();

    if cli.all_selected {
        return view
            .providers
            .into_iter()
            .filter_map(|provider| {
                let selected_model = provider
                    .selected_model_id
                    .as_deref()
                    .and_then(|selected| provider.models.iter().find(|model| model.id == selected))
                    .or_else(|| provider.models.first())?;
                Some(ProbeTarget {
                    provider_id: provider.id,
                    provider_name: provider.name,
                    model_id: selected_model.id.clone(),
                    model_name: selected_model.name.clone(),
                    model_api_name: selected_model.model.clone(),
                })
            })
            .collect();
    }

    if let Some(provider_id) = cli.provider_id.as_deref() {
        let provider = view
            .providers
            .iter()
            .find(|candidate| candidate.id == provider_id);
        let Some(provider) = provider else {
            return Vec::new();
        };
        let model = cli
            .model_id
            .as_deref()
            .and_then(|selected| {
                provider
                    .models
                    .iter()
                    .find(|candidate| candidate.id == selected)
            })
            .or_else(|| {
                provider.selected_model_id.as_deref().and_then(|selected| {
                    provider
                        .models
                        .iter()
                        .find(|candidate| candidate.id == selected)
                })
            })
            .or_else(|| provider.models.first());
        let Some(model) = model else {
            return Vec::new();
        };
        return vec![ProbeTarget {
            provider_id: provider.id.clone(),
            provider_name: provider.name.clone(),
            model_id: model.id.clone(),
            model_name: model.name.clone(),
            model_api_name: model.model.clone(),
        }];
    }

    let selection =
        registry.resolve_provider_selection(cli.provider_id.as_deref(), cli.model_id.as_deref());
    let provider = view.providers.iter().find(|candidate| {
        candidate.name == selection.provider_name
            || Some(candidate.id.as_str()) == cli.provider_id.as_deref()
    });
    let model = provider.and_then(|provider_item| {
        provider_item.models.iter().find(|candidate| {
            candidate.model == selection.model
                || Some(candidate.id.as_str()) == cli.model_id.as_deref()
        })
    });

    vec![ProbeTarget {
        provider_id: provider
            .map(|candidate| candidate.id.clone())
            .unwrap_or_else(|| {
                cli.provider_id
                    .clone()
                    .unwrap_or_else(|| selection.provider_name.clone())
            }),
        provider_name: selection.provider_name,
        model_id: model
            .map(|candidate| candidate.id.clone())
            .unwrap_or_else(|| {
                cli.model_id
                    .clone()
                    .unwrap_or_else(|| selection.model.clone())
            }),
        model_name: model
            .map(|candidate| candidate.name.clone())
            .unwrap_or_else(|| selection.model.clone()),
        model_api_name: selection.model,
    }]
}

fn run_probe(target: &ProbeTarget, prompt: &str, run_index: usize) {
    let mut runtime = AgentRuntime::new();
    let started_at = Instant::now();
    let sink = RecordingTurnEventSink::new(started_at);
    let run_nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let turn_id = format!("trace-redteam-{}-{}", run_index, run_nonce);
    let session_id = format!("trace-redteam-probe-{}-{}", target.provider_id, run_nonce);

    runtime.start_turn_stream(
        &sink,
        turn_id.clone(),
        TurnInput {
            message: prompt.to_string(),
            display_message: None,
            provider_id: Some(target.provider_id.clone()),
            model_id: Some(target.model_id.clone()),
            reasoning_effort: None,
            session_id: Some(session_id.clone()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        },
    );

    let observed_wall_ms = started_at.elapsed().as_millis() as u64;
    let events = sink.snapshot();
    let first_visible_event = events.iter().find(|event| is_visible_event(event));
    let terminal_event = events.iter().rev().find(|event| is_terminal_event(event));

    let reported_ttft_ms = terminal_event
        .and_then(|event| event.payload.first_token_latency_ms)
        .or_else(|| first_visible_event.and_then(|event| event.payload.first_token_latency_ms))
        .unwrap_or_default();
    let observed_ttft_ms = first_visible_event
        .map(|event| event.observed_at_ms)
        .unwrap_or_default();
    let turn_duration_ms = terminal_event
        .and_then(|event| event.payload.turn_duration_ms)
        .unwrap_or(observed_wall_ms);
    let output_tokens = terminal_event
        .and_then(|event| event.payload.output_tokens)
        .unwrap_or_default();

    let reported_active_generation_ms = if reported_ttft_ms > 0 {
        turn_duration_ms.saturating_sub(reported_ttft_ms).max(1)
    } else {
        turn_duration_ms.max(1)
    };
    let observed_active_generation_ms = if observed_ttft_ms > 0 {
        turn_duration_ms.saturating_sub(observed_ttft_ms).max(1)
    } else {
        turn_duration_ms.max(1)
    };

    let reported_token_per_second = if output_tokens > 0 {
        output_tokens as f64 / (reported_active_generation_ms as f64 / 1000.0)
    } else {
        0.0
    };
    let observed_token_per_second = if output_tokens > 0 {
        output_tokens as f64 / (observed_active_generation_ms as f64 / 1000.0)
    } else {
        0.0
    };
    let token_per_second_bias_pct = if observed_token_per_second > 0.0 {
        ((reported_token_per_second - observed_token_per_second) / observed_token_per_second)
            * 100.0
    } else {
        0.0
    };

    println!("---");
    println!("run={}", run_index);
    println!("provider_id={}", target.provider_id);
    println!("provider={}", target.provider_name);
    println!("model_id={}", target.model_id);
    println!("model_name={}", target.model_name);
    println!("model={}", target.model_api_name);
    println!("session_id={}", session_id);
    println!(
        "first_visible_event={}",
        first_visible_event
            .map(|event| event.name.as_str())
            .unwrap_or("none")
    );
    println!(
        "terminal_event={}",
        terminal_event
            .map(|event| event.name.as_str())
            .unwrap_or("none")
    );
    println!("reported_ttft_ms={}", reported_ttft_ms);
    println!("observed_ttft_ms={}", observed_ttft_ms);
    println!(
        "ttft_bias_ms={}",
        observed_ttft_ms as i64 - reported_ttft_ms as i64
    );
    println!("turn_duration_ms={}", turn_duration_ms);
    println!("observed_wall_ms={}", observed_wall_ms);
    println!("output_tokens={}", output_tokens);
    println!("reported_token_per_second={:.2}", reported_token_per_second);
    println!("observed_token_per_second={:.2}", observed_token_per_second);
    println!("token_per_second_bias_pct={:.2}", token_per_second_bias_pct);
    println!("event_count={}", events.len());
    for event in events.iter().take(8) {
        println!(
            "event={} observed_at_ms={} text_len={} reasoning_len={} event_ttft_ms={}",
            event.name,
            event.observed_at_ms,
            event.payload.text.as_deref().map(str::len).unwrap_or(0),
            event
                .payload
                .reasoning_content
                .as_deref()
                .map(str::len)
                .unwrap_or(0),
            event.payload.first_token_latency_ms.unwrap_or_default()
        );
    }
}

fn is_terminal_event(event: &TimedTurnEvent) -> bool {
    matches!(
        event.name.as_str(),
        "turn:completed" | "turn:failed" | "turn:cancelled"
    )
}

fn is_visible_event(event: &TimedTurnEvent) -> bool {
    match event.name.as_str() {
        "turn:delta" => has_visible_payload(&event.payload),
        "turn:completed" | "turn:failed" | "turn:cancelled" => has_visible_payload(&event.payload),
        _ => false,
    }
}

fn has_visible_payload(payload: &TurnStreamEvent) -> bool {
    has_text(payload.text.as_deref())
        || has_text(payload.reasoning_content.as_deref())
        || has_text(payload.error.as_deref())
}

fn has_text(value: Option<&str>) -> bool {
    value.map(|text| !text.trim().is_empty()).unwrap_or(false)
}
