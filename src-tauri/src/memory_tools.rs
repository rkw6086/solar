use super::RunContext;
use crate::{
    engine::Engine,
    model::{Memory, id, now},
};
use serde::Deserialize;
use serde_json::Value;
use solar_tools::ToolOutput;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveMemoryArgs {
    content: String,
    memory_id: Option<String>,
    scope: Option<String>,
    #[serde(default)]
    orbit_ids: Vec<String>,
    #[serde(default)]
    project_ids: Vec<String>,
    // Compatibility with existing agents: true means current project.
    shared: Option<bool>,
}
impl Engine {
    pub(super) fn memories_for_task(&self, ctx: &RunContext) -> Result<Vec<Memory>, String> {
        let projects: Vec<_> = self
            .accessible_projects(ctx)?
            .into_iter()
            .map(|p| p.id)
            .collect();
        Ok(self
            .store
            .list::<Memory>("memories")?
            .into_iter()
            .filter(|memory| {
                ctx.ancestry
                    .iter()
                    .all(|orbit_id| memory.visible_to(orbit_id, &projects))
            })
            .collect())
    }

    pub(super) fn save_task_memory(
        &self,
        ctx: &RunContext,
        value: &Value,
    ) -> Result<ToolOutput, String> {
        let args: SaveMemoryArgs =
            serde_json::from_value(value.clone()).map_err(|e| format!("Invalid memory: {e}"))?;
        if (!args.orbit_ids.is_empty() && !args.project_ids.is_empty())
            || (args.scope.as_deref() == Some("system")
                && (!args.orbit_ids.is_empty() || !args.project_ids.is_empty()))
            || (args.scope.as_deref() == Some("orbit") && !args.project_ids.is_empty())
            || (args.scope.as_deref() == Some("project") && !args.orbit_ids.is_empty())
        {
            return Err("Memory scope and audience IDs must agree".into());
        }
        let mut memory = if let Some(memory_id) = &args.memory_id {
            self.memories_for_task(ctx)?
                .into_iter()
                .find(|memory| &memory.id == memory_id)
                .ok_or("Memory not found or unavailable to this task")?
        } else {
            Memory {
                id: id(),
                orbit_ids: vec![ctx.task.orbit_id.clone()],
                project_ids: vec![],
                orbit_id: None,
                project_id: None,
                content: String::new(),
                created_at: now(),
            }
        };
        let reassigned = args.scope.is_some()
            || !args.orbit_ids.is_empty()
            || !args.project_ids.is_empty()
            || args.shared.is_some();
        if reassigned {
            let scope = args.scope.as_deref().unwrap_or(
                if args.shared == Some(true) || !args.project_ids.is_empty() {
                    "project"
                } else {
                    "orbit"
                },
            );
            memory.orbit_id = None;
            memory.project_id = None;
            memory.orbit_ids.clear();
            memory.project_ids.clear();
            match scope {
                "system" => {}
                "orbit" => {
                    memory.orbit_ids = if args.orbit_ids.is_empty() {
                        vec![ctx.task.orbit_id.clone()]
                    } else {
                        args.orbit_ids
                    }
                }
                "project" => {
                    memory.project_ids = if args.project_ids.is_empty() {
                        vec![
                            ctx.conversation
                                .project_id
                                .clone()
                                .ok_or("Choose an accessible project ID for this memory")?,
                        ]
                    } else {
                        args.project_ids
                    }
                }
                _ => return Err("Memory scope must be system, orbit, or project".into()),
            }
        }
        let projects: Vec<_> = self
            .accessible_projects(ctx)?
            .into_iter()
            .map(|p| p.id)
            .collect();
        if memory.project_ids().iter().any(|id| !projects.contains(id)) {
            return Err(
                "Every project attached to this memory must be available to this task".into(),
            );
        }
        let orbit_ids = memory.orbit_ids();
        if !orbit_ids.is_empty() && !orbit_ids.contains(&ctx.task.orbit_id) {
            return Err("Shared Sprite memories must include the Sprite saving them".into());
        }
        if ctx.ancestry.len() > 1
            && (orbit_ids != vec![ctx.task.orbit_id.clone()] || !memory.project_ids().is_empty())
        {
            return Err(
                "Delegated tasks may only save private notes for the current Sprite".into(),
            );
        }
        memory.content = args.content;
        let saved = self.save_memory(memory)?;
        super::output(serde_json::to_string(&saved).map_err(|e| e.to_string())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Project, Task};
    use serde_json::json;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn context(engine: &Arc<Engine>, orbit: &str) -> RunContext {
        let conversation = engine
            .create_conversation(vec![orbit.into()], orbit.into(), "Memory test".into(), None)
            .unwrap();
        RunContext {
            task: Task {
                id: id(),
                conversation_id: conversation.id.clone(),
                orbit_id: orbit.into(),
                status: "running".into(),
                summary: String::new(),
                error: None,
                created_at: now(),
                user_message_id: None,
            },
            conversation,
            origin: orbit.into(),
            ancestry: vec![orbit.into()],
            history: vec![],
            token: CancellationToken::new(),
            user_message_id: None,
            model_origin: None,
        }
    }
    #[test]
    fn shared_memories_are_visible_updatable_and_scope_checked() {
        let root = tempfile::tempdir().unwrap();
        let project_folder = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let mut other = engine.orbit("assistant").unwrap();
        other.id = "other".into();
        engine.save_orbit(other).unwrap();
        let first = context(&engine, "assistant");
        let second = context(&engine, "other");
        let save = engine
            .save_task_memory(
                &first,
                &json!({"scope":"system","content":"Use friendly explanations"}),
            )
            .unwrap();
        let global: Memory = serde_json::from_str(&save.content).unwrap();
        assert!(
            engine
                .memories_for_task(&second)
                .unwrap()
                .iter()
                .any(|m| m.id == global.id)
        );
        engine
            .save_task_memory(
                &second,
                &json!({"memoryId":global.id,"content":"Use concise, friendly explanations"}),
            )
            .unwrap();
        assert_eq!(
            engine
                .store
                .get::<Memory>("memories", &global.id)
                .unwrap()
                .unwrap()
                .content,
            "Use concise, friendly explanations"
        );
        engine.save_task_memory(&first, &json!({"scope":"orbit","orbitIds":["assistant","other"],"content":"Shared preference"})).unwrap();
        engine
            .save_task_memory(&first, &json!({"content":"Private preference"}))
            .unwrap();
        assert_eq!(engine.memories_for_task(&second).unwrap().len(), 2);
        engine
            .save_project(Project {
                id: "secret".into(),
                name: "Secret".into(),
                directory: project_folder.path().display().to_string(),
                description: String::new(),
            })
            .unwrap();
        assert!(
            engine
                .save_task_memory(
                    &first,
                    &json!({"scope":"project","projectIds":["secret"],"content":"Forbidden"})
                )
                .is_err()
        );
        let mut delegated = first;
        delegated.ancestry.push("other".into());
        assert!(
            engine
                .save_task_memory(
                    &delegated,
                    &json!({"scope":"system","content":"Do not publish this"})
                )
                .is_err()
        );
        assert!(engine.save_task_memory(&delegated, &json!({"scope":"orbit","orbitIds":["assistant","other"],"content":"Do not broaden"})).is_err());
    }

    #[test]
    fn project_memories_can_share_across_multiple_authorized_projects() {
        let root = tempfile::tempdir().unwrap();
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        for (id, path) in [("one", one.path()), ("two", two.path())] {
            engine
                .save_project(Project {
                    id: id.into(),
                    name: id.into(),
                    directory: path.display().to_string(),
                    description: String::new(),
                })
                .unwrap();
        }
        let mut orbit = engine.orbit("assistant").unwrap();
        orbit.project_ids = vec!["one".into(), "two".into()];
        engine.save_orbit(orbit.clone()).unwrap();
        orbit.id = "other".into();
        orbit.project_ids = vec!["two".into()];
        engine.save_orbit(orbit).unwrap();
        let first = context(&engine, "assistant");
        let second = context(&engine, "other");
        engine.save_task_memory(&first, &json!({"scope":"project","projectIds":["one","two"],"content":"Shared architecture"})).unwrap();
        assert_eq!(engine.memories_for_task(&second).unwrap().len(), 1);
        let shared = engine.memories_for_task(&second).unwrap().remove(0);
        assert!(engine.save_task_memory(&second, &json!({"memoryId":shared.id,"content":"Cannot modify the inaccessible project too"})).is_err());
    }
    #[test]
    fn delegated_memory_reads_require_every_ancestor_and_writes_stay_private() {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let mut other = engine.orbit("assistant").unwrap();
        other.id = "other".into();
        engine.save_orbit(other).unwrap();
        let parent = context(&engine, "assistant");
        let child = context(&engine, "other");
        engine
            .save_task_memory(&child, &json!({"content":"Child private fact"}))
            .unwrap();
        engine
            .save_task_memory(
                &parent,
                &json!({"scope":"orbit","orbitIds":["assistant","other"],"content":"Joint fact"}),
            )
            .unwrap();
        let system: Memory = serde_json::from_str(
            &engine
                .save_task_memory(&parent, &json!({"scope":"system","content":"Public fact"}))
                .unwrap()
                .content,
        )
        .unwrap();
        let mut delegated = child;
        delegated.ancestry = vec!["assistant".into(), "other".into()];
        let visible = engine.memories_for_task(&delegated).unwrap();
        assert_eq!(visible.len(), 2);
        assert!(!visible.iter().any(|m| m.content == "Child private fact"));
        assert!(
            engine
                .save_task_memory(
                    &delegated,
                    &json!({"memoryId":system.id,"content":"Do not publish delegated work"})
                )
                .is_err()
        );
        assert!(
            engine
                .save_task_memory(&delegated, &json!({"content":"A private child note"}))
                .is_ok()
        );
    }
}
