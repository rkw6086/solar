use crate::{engine::Engine, model::Orbit};

pub(crate) fn add_assistant(engine: &Engine) {
    engine
        .save_orbit(Orbit {
            id: "assistant".into(),
            name: "Sol".into(),
            role: "Personal assistant".into(),
            tasks: String::new(),
            personality: "Thoughtful and precise".into(),
            instructions: String::new(),
            color: "#f5b45b".into(),
            model: String::new(),
            provider_id: String::new(),
            autonomy: "workspace".into(),
            tool_grants: ["files", "web", "memory", "collaboration"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            project_ids: vec![],
            x: None,
            y: None,
            enabled: true,
        })
        .unwrap();
}
