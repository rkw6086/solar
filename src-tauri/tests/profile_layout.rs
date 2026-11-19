mod common;

use solar::{
    engine::Engine,
    model::{Memory, Orbit, Settings},
    store::Store,
};
use std::fs;

fn configured_orbit() -> Orbit {
    Orbit {
        id: "custom".into(),
        name: "My Assistant".into(),
        role: "Assistant".into(),
        tasks: String::new(),
        personality: "Thoughtful".into(),
        instructions: String::new(),
        color: "#ffffff".into(),
        model: String::new(),
        provider_id: String::new(),
        autonomy: "ask".into(),
        tool_grants: vec![],
        project_ids: vec![],
        x: None,
        y: None,
        enabled: true,
    }
}

#[test]
fn preconfigured_profiles_are_used_without_adding_starter_orbits() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::open_profile(temporary.path()).unwrap();
    let settings = Settings {
        sound_effects_volume: 0.08,
        ..Settings::default()
    };
    store.put("settings", "app", &settings).unwrap();
    store.put("orbits", "custom", &configured_orbit()).unwrap();
    let directory = store.orbit_dir("custom").unwrap();
    drop(store);

    let engine = Engine::open(temporary.path().into()).unwrap();
    let state = engine.bootstrap().unwrap();
    assert_eq!(state.orbits.len(), 1);
    assert_eq!(state.orbits[0].id, "custom");
    assert_eq!(state.settings.sound_effects_volume, 0.08);
    for child in [
        "config.toml",
        "orbit.sqlite",
        "workspace",
        "skills",
        "memory",
    ] {
        assert!(directory.join(child).exists(), "missing {child}");
    }
}

#[test]
fn manual_toml_edits_are_authoritative_after_restart() {
    let temporary = tempfile::tempdir().unwrap();
    let engine = Engine::open(temporary.path().into()).unwrap();
    common::add_assistant(&engine);
    drop(engine);
    let path = temporary.path().join("config.toml");
    let mut master: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    master["settings"]["maxConcurrent"] = toml::Value::Integer(3);
    master["settings"]["soundEffectsVolume"] = toml::Value::Float(0.07);
    fs::write(&path, toml::to_string_pretty(&master).unwrap()).unwrap();
    let path = temporary.path().join("orbits/sol/config.toml");
    let mut profile: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    profile["orbit"]["personality"] = toml::Value::String("Patient and precise".into());
    fs::write(&path, toml::to_string_pretty(&profile).unwrap()).unwrap();

    let engine = Engine::open(temporary.path().into()).unwrap();
    assert_eq!(engine.settings().unwrap().max_concurrent, 3);
    assert_eq!(engine.settings().unwrap().sound_effects_volume, 0.07);
    assert_eq!(
        engine.orbit("assistant").unwrap().personality,
        "Patient and precise"
    );
}

#[test]
fn private_memory_is_in_the_orbit_database_and_survives_a_display_rename() {
    let temporary = tempfile::tempdir().unwrap();
    let engine = Engine::open(temporary.path().into()).unwrap();
    common::add_assistant(&engine);
    engine
        .save_memory(Memory {
            orbit_ids: Vec::new(),
            project_ids: Vec::new(),
            id: "private-note".into(),
            orbit_id: Some("assistant".into()),
            project_id: None,
            content: "The user prefers short updates".into(),
            created_at: String::new(),
        })
        .unwrap();
    let mut orbit = engine.orbit("assistant").unwrap();
    orbit.name = "Renamed Assistant".into();
    engine.save_orbit(orbit).unwrap();
    drop(engine);

    let main = rusqlite::Connection::open(temporary.path().join("solar.sqlite")).unwrap();
    let private =
        rusqlite::Connection::open(temporary.path().join("orbits/sol/orbit.sqlite")).unwrap();
    let count = |connection: &rusqlite::Connection| {
        connection
            .query_row(
                "SELECT count(*) FROM documents WHERE kind='memories' AND id='private-note'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(count(&main), 0);
    assert_eq!(count(&private), 1);
    drop(private);
    drop(main);
    let engine = Engine::open(temporary.path().into()).unwrap();
    assert_eq!(engine.orbit("assistant").unwrap().name, "Renamed Assistant");
    assert_eq!(engine.memories(Some("assistant"), None).unwrap().len(), 1);
}

#[test]
fn fresh_profile_stays_empty_until_an_orbit_is_created() {
    let temporary = tempfile::tempdir().unwrap();
    let engine = Engine::open(temporary.path().into()).unwrap();
    let state = engine.bootstrap().unwrap();
    assert!(state.orbits.is_empty());
    assert!(state.conversations.is_empty());
    assert!(state.providers.is_empty());
    assert!(!state.settings.onboarding_complete);
    assert_eq!(
        fs::read_dir(temporary.path().join("orbits"))
            .unwrap()
            .count(),
        0
    );
    drop(engine);

    let engine = Engine::open(temporary.path().into()).unwrap();
    assert!(engine.bootstrap().unwrap().orbits.is_empty());
    assert!(engine.bootstrap().unwrap().conversations.is_empty());
    common::add_assistant(&engine);
    assert_eq!(engine.bootstrap().unwrap().orbits.len(), 1);
    assert!(engine.bootstrap().unwrap().conversations.is_empty());
    drop(engine);

    let engine = Engine::open(temporary.path().into()).unwrap();
    assert_eq!(engine.bootstrap().unwrap().orbits.len(), 1);
    engine.delete_orbit("assistant").unwrap();
    drop(engine);
    let engine = Engine::open(temporary.path().into()).unwrap();
    assert!(engine.bootstrap().unwrap().orbits.is_empty());
    assert!(engine.bootstrap().unwrap().conversations.is_empty());
}
