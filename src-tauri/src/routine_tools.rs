use super::RunContext;
use crate::{engine::Engine, model::Routine};
use serde::Deserialize;
use serde_json::{Value, json};
use solar_agent_core::ToolDefinition;
use solar_tools::ToolOutput;
use std::sync::Arc;

pub(super) fn definitions() -> Vec<ToolDefinition> {
    let fields = json!({
        "prompt": {"type":"string","minLength":1,"maxLength":16000,"description":"The instructions this Orbit should follow each time the routine runs."},
        "trigger": {"type":"string","enum":["interval","folder","completion"],"description":"interval: repeat after the given minutes; folder: run when the project folder changes; completion: run after a direct user task finishes successfully. Routine runs do not trigger completion routines again."},
        "intervalMinutes": {"type":"integer","minimum":1,"maximum":43200,"description":"Minutes between interval runs. Defaults to 60 on creation."},
        "enabled": {"type":"boolean","description":"Enable or pause automatic runs. Defaults to true on creation."},
        "projectId": {"type":["string","null"],"description":"An accessible project ID from project_list. Required for folder triggers. Null means a private routine; omit on update to preserve the project."}
    });
    let mut update_fields = fields.as_object().expect("object").clone();
    update_fields.insert("routineId".into(), json!({"type":"string"}));
    [
        (
            "routine_list",
            "List your own routines, including IDs, instructions, trigger, timing, enabled state, project, and last run. Includes paused routines. Only routines within this task's project access are visible. Routines run while Solar is open and the computer is awake.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
        ),
        (
            "routine_create",
            "Create a recurring or event-triggered routine for yourself. Check routine_list first to avoid duplicates. Supports intervals, project folder changes, or successful direct-task completion. Available in direct conversations; delegated tasks must ask the user to schedule with the responsible Orbit.",
            json!({"type":"object","properties":fields,"required":["prompt","trigger"],"additionalProperties":false}),
        ),
        (
            "routine_update",
            "Update your own routine by ID. Only supplied fields change; use enabled=false to pause, enabled=true to resume, or projectId=null to remove the project. Available in direct conversations; delegated tasks cannot change routines.",
            json!({"type":"object","properties":update_fields,"required":["routineId"],"additionalProperties":false}),
        ),
        (
            "routine_delete",
            "Delete your own routine by ID, stopping future automatic runs. Already queued or running tasks continue. Available in direct conversations; delegated tasks cannot delete routines.",
            json!({"type":"object","properties":{"routineId":{"type":"string"}},"required":["routineId"],"additionalProperties":false}),
        ),
        (
            "routine_run",
            "Queue one run of your own routine now, even if paused, in a separate conversation. Returns the conversation ID; queued does not mean completed. Does not enable a paused routine or trigger other completion routines. Available in direct conversations; delegated tasks cannot launch routines.",
            json!({"type":"object","properties":{"routineId":{"type":"string"}},"required":["routineId"],"additionalProperties":false}),
        ),
    ]
    .into_iter()
    .map(|(name, description, input_schema)| ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema,
    })
    .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateArgs {
    prompt: String,
    trigger: String,
    #[serde(default = "default_interval")]
    interval_minutes: u32,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    project_id: Option<String>,
}

fn default_interval() -> u32 {
    60
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateArgs {
    routine_id: String,
    prompt: Option<String>,
    trigger: Option<String>,
    interval_minutes: Option<u32>,
    enabled: Option<bool>,
    #[serde(default, deserialize_with = "project_change")]
    project_id: Option<Option<String>>,
}

// Missing preserves the existing project; an explicit null removes it.
fn project_change<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdArgs {
    routine_id: String,
}

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone())
        .map_err(|error| format!("Invalid routine arguments: {error}"))
}

fn validate_interval(minutes: u32) -> Result<(), String> {
    if !(1..=43_200).contains(&minutes) {
        return Err("Routine interval must be between 1 and 43,200 minutes".into());
    }
    Ok(())
}

fn direct_task(ctx: &RunContext) -> Result<(), String> {
    // A scheduled task starts a new root run. Until routines can persist inherited
    // scope, delegation must not turn restricted work into unrestricted future work.
    if ctx.ancestry.len() > 1 {
        return Err("Delegated tasks cannot change or run routines. Ask the user to schedule this in a direct conversation with the responsible Orbit.".into());
    }
    Ok(())
}

fn visible_routine(routine: &Routine, ctx: &RunContext, projects: &[String]) -> bool {
    routine.orbit_id == ctx.task.orbit_id
        && routine
            .project_id
            .as_ref()
            .is_none_or(|id| projects.contains(id))
}

fn check_scope(routine: &Routine, ctx: &RunContext, projects: &[String]) -> Result<(), String> {
    if !visible_routine(routine, ctx, projects) {
        return Err("Routine not found or not available to this Orbit".into());
    }
    Ok(())
}

fn routine_view(routine: &Routine) -> Value {
    // Folder signatures can contain tens of thousands of paths. They are internal
    // scheduler state, not useful context for the model or user.
    json!({
        "id": routine.id,
        "orbitId": routine.orbit_id,
        "prompt": routine.prompt,
        "trigger": routine.trigger,
        "intervalMinutes": routine.interval_minutes,
        "enabled": routine.enabled,
        "projectId": routine.project_id,
        "lastRunAt": routine.last_run_at,
    })
}

fn result(value: Value) -> Result<ToolOutput, String> {
    super::output(serde_json::to_string(&value).map_err(|error| error.to_string())?)
}

impl Engine {
    pub(super) fn execute_routine_tool(
        self: &Arc<Self>,
        ctx: &RunContext,
        name: &str,
        args: &Value,
    ) -> Result<ToolOutput, String> {
        let projects: Vec<String> = self
            .accessible_projects(ctx)?
            .into_iter()
            .map(|project| project.id)
            .collect();
        match name {
            "routine_list" => {
                let _: ListArgs = parse(args)?;
                let mut routines = self.store.list::<Routine>("routines")?;
                routines.retain(|routine| visible_routine(routine, ctx, &projects));
                routines.sort_by(|a, b| a.id.cmp(&b.id));
                result(Value::Array(routines.iter().map(routine_view).collect()))
            }
            "routine_create" => {
                direct_task(ctx)?;
                let args: CreateArgs = parse(args)?;
                validate_interval(args.interval_minutes)?;
                let routine = Routine {
                    id: String::new(),
                    orbit_id: ctx.task.orbit_id.clone(),
                    prompt: args.prompt,
                    trigger: args.trigger,
                    interval_minutes: args.interval_minutes,
                    enabled: args.enabled,
                    project_id: args.project_id,
                    last_run_at: None,
                    observed_signature: None,
                };
                check_scope(&routine, ctx, &projects)?;
                result(routine_view(&self.save_routine(routine)?))
            }
            "routine_update" => {
                direct_task(ctx)?;
                let args: UpdateArgs = parse(args)?;
                if args.prompt.is_none()
                    && args.trigger.is_none()
                    && args.interval_minutes.is_none()
                    && args.enabled.is_none()
                    && args.project_id.is_none()
                {
                    return Err("Supply at least one routine field to update".into());
                }
                if let Some(minutes) = args.interval_minutes {
                    validate_interval(minutes)?;
                }
                let routine = self.update_routine_with(&args.routine_id, |routine| {
                    check_scope(routine, ctx, &projects)?;
                    if let Some(prompt) = args.prompt {
                        routine.prompt = prompt;
                    }
                    if let Some(trigger) = args.trigger {
                        routine.trigger = trigger;
                    }
                    if let Some(minutes) = args.interval_minutes {
                        routine.interval_minutes = minutes;
                    }
                    if let Some(enabled) = args.enabled {
                        routine.enabled = enabled;
                    }
                    if let Some(project_id) = args.project_id {
                        routine.project_id = project_id;
                    }
                    check_scope(routine, ctx, &projects)
                })?;
                result(routine_view(&routine))
            }
            "routine_delete" => {
                direct_task(ctx)?;
                let args: IdArgs = parse(args)?;
                self.delete_routine_checked(&args.routine_id, |routine| {
                    check_scope(routine, ctx, &projects)
                })?;
                result(json!({"routineId":args.routine_id,"deleted":true}))
            }
            "routine_run" => {
                direct_task(ctx)?;
                let args: IdArgs = parse(args)?;
                let conversation = self.run_routine_checked(&args.routine_id, |routine| {
                    check_scope(routine, ctx, &projects)
                })?;
                result(
                    json!({"routineId":args.routine_id,"conversationId":conversation.id,"status":"queued"}),
                )
            }
            _ => Err("Unknown routine tool".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_updates_distinguish_absent_from_explicit_null() {
        let absent: UpdateArgs = parse(&json!({"routineId":"test","enabled":false})).unwrap();
        let null: UpdateArgs = parse(&json!({"routineId":"test","projectId":null})).unwrap();
        let value: UpdateArgs = parse(&json!({"routineId":"test","projectId":"project"})).unwrap();
        assert_eq!(absent.project_id, None);
        assert_eq!(null.project_id, Some(None));
        assert_eq!(value.project_id, Some(Some("project".into())));
    }

    #[test]
    fn creation_rejects_owner_and_scheduler_state_injection() {
        for (key, value) in [
            ("orbitId", json!("someone-else")),
            ("id", json!("existing-routine")),
            ("lastRunAt", json!("1970-01-01T00:00:00Z")),
            ("observedSignature", json!("fake")),
        ] {
            let mut args = json!({"prompt":"Check the build","trigger":"interval"});
            args[key] = value;
            assert!(parse::<CreateArgs>(&args).is_err());
        }
        assert!(
            parse::<CreateArgs>(
                &json!({"prompt":"Check","trigger":"interval","intervalMinutes":-1})
            )
            .is_err()
        );
        assert!(validate_interval(0).is_err());
        assert!(validate_interval(43_201).is_err());
    }
}
