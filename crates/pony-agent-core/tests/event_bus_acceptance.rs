use pony_agent_core::agent::event_bus::{AgentEvent, AgentEventBus};

#[test]
fn test_agent_event_bus_publish_and_subscribe() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let bus = AgentEventBus::new(16);
        let mut rx = bus.subscribe();

        let event = AgentEvent::SessionCreated {
            session_id: "sess-123".to_string(),
        };

        let count = bus.publish(event.clone()).expect("publish should succeed");
        assert_eq!(count, 1);

        let received = rx.recv().await.expect("receive should succeed");
        assert_eq!(received, event);
    });
}

#[test]
fn test_agent_event_bus_bounded_lag() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let bus = AgentEventBus::new(2);
        let mut rx = bus.subscribe();

        for i in 0..5 {
            let _ = bus.publish(AgentEvent::Custom {
                topic: format!("t{}", i),
                data: serde_json::json!({ "i": i }),
            });
        }

        match rx.recv().await {
            Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                assert!(missed > 0);
            }
            other => panic!("expected lagged error due to capacity 2, got {:?}", other),
        }
    });
}
