use std::collections::HashMap;
use std::path::PathBuf;

/// Visibility of a context layer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LayerVisibility {
    /// Agent-only, hidden from user
    #[serde(rename = "internal")]
    Internal,
    /// User-visible
    #[serde(rename = "visible")]
    Visible,
}

/// Condition for a conditional context layer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayerCondition {
    /// Current workflow state matches exactly.
    #[serde(rename = "state_matches")]
    StateMatches { state: String },
    /// Variable is set to a specific value.
    #[serde(rename = "variable_set")]
    VariableSet { key: String, value: String },
}

/// Source of a context layer's content.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextSource {
    /// Inline template string with `{{ var }}` placeholders.
    #[serde(rename = "template")]
    Template { template: String },
    /// Read content from a file, with optional template rendering.
    #[serde(rename = "file")]
    File { path: PathBuf },
    /// Conditional wrapper that includes inner source only when condition matches.
    #[serde(rename = "conditional")]
    Conditional {
        condition: LayerCondition,
        source: Box<ContextSource>,
    },
}

/// A single context layer definition.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextLayer {
    pub id: String,
    pub priority: u8,
    pub visibility: LayerVisibility,
    pub source: ContextSource,
}

/// Assembled context output split by visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledContext {
    /// Agent-only instructions, hidden from user.
    pub internal_instructions: Vec<String>,
    /// User-visible content.
    pub visible_content: Vec<String>,
}

/// Assembles context layers into ordered, visibility-separated output streams.
pub struct ContextAssembler {
    layers: Vec<ContextLayer>,
}

impl ContextAssembler {
    pub fn new(layers: Vec<ContextLayer>) -> Self {
        Self { layers }
    }

    pub fn assemble(&self, state: &str, variables: &HashMap<String, String>) -> AssembledContext {
        let mut internal: Vec<(u8, String)> = Vec::new();
        let mut visible: Vec<(u8, String)> = Vec::new();

        for layer in &self.layers {
            // Resolve conditional layers — skip if condition does not match.
            let effective_source = match &layer.source {
                ContextSource::Conditional { condition, source } => {
                    if !evaluate_condition(condition, state, variables) {
                        continue;
                    }
                    source
                }
                other => other,
            };

            // Render the layer content.
            let content = match effective_source {
                ContextSource::Template { template } => render_template(template, state, variables),
                ContextSource::File { path } => {
                    // Read file content, then apply template rendering so file
                    // layers support the same `{{ var }}` substitution. Failures
                    // are logged and the layer is skipped (best-effort), matching
                    // the original intent — but no longer silently dropped.
                    match std::fs::read_to_string(path) {
                        Ok(raw) => render_template(&raw, state, variables),
                        Err(e) => {
                            tracing::warn!(
                                layer = %layer.id,
                                path = %path.display(),
                                error = %e,
                                "Failed to read context layer file; skipping layer"
                            );
                            continue;
                        }
                    }
                }
                ContextSource::Conditional { .. } => {
                    // Already unwrapped above — unreachable.
                    continue;
                }
            };

            match layer.visibility {
                LayerVisibility::Internal => internal.push((layer.priority, content)),
                LayerVisibility::Visible => visible.push((layer.priority, content)),
            }
        }

        // Sort ascending by priority so higher-priority layers appear later
        // (closer to the current turn in the context window).
        internal.sort_by_key(|(p, _)| *p);
        visible.sort_by_key(|(p, _)| *p);

        AssembledContext {
            internal_instructions: internal.into_iter().map(|(_, c)| c).collect(),
            visible_content: visible.into_iter().map(|(_, c)| c).collect(),
        }
    }
}

fn evaluate_condition(
    condition: &LayerCondition,
    state: &str,
    variables: &HashMap<String, String>,
) -> bool {
    match condition {
        LayerCondition::StateMatches { state: expected } => state == expected,
        LayerCondition::VariableSet {
            key,
            value: expected_value,
        } => variables
            .get(key)
            .map(|v| v == expected_value)
            .unwrap_or(false),
    }
}

fn render_template(template: &str, state: &str, variables: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    result = result.replace("{{ state }}", state);
    for (key, value) in variables {
        let placeholder = format!("{{{{ {} }}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_layer_creation_and_serde_roundtrip() {
        let layer = ContextLayer {
            id: "phase-instruction".to_string(),
            priority: 35,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Template {
                template: "当前处于 {{ state }} 阶段".to_string(),
            },
        };

        assert_eq!(layer.id, "phase-instruction");
        assert_eq!(layer.priority, 35);
        assert_eq!(layer.visibility, LayerVisibility::Internal);

        let json = serde_json::to_string(&layer).expect("serialize");
        let roundtripped: ContextLayer = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(roundtripped, layer);
    }

    #[test]
    fn test_internal_visibility_hidden_from_visible_stream() {
        let layers = vec![ContextLayer {
            id: "secret".to_string(),
            priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Template {
                template: "internal only".to_string(),
            },
        }];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("open", &HashMap::new());

        assert_eq!(ctx.internal_instructions.len(), 1);
        assert_eq!(ctx.internal_instructions[0], "internal only");
        assert!(ctx.visible_content.is_empty());
    }

    #[test]
    fn test_visible_layer_appears_to_user() {
        let layers = vec![ContextLayer {
            id: "greeting".to_string(),
            priority: 10,
            visibility: LayerVisibility::Visible,
            source: ContextSource::Template {
                template: "Hello user!".to_string(),
            },
        }];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("open", &HashMap::new());

        assert!(ctx.internal_instructions.is_empty());
        assert_eq!(ctx.visible_content.len(), 1);
        assert_eq!(ctx.visible_content[0], "Hello user!");
    }

    #[test]
    fn test_priority_ordering_low_to_high() {
        let layers = vec![
            ContextLayer {
                id: "high".to_string(),
                priority: 50,
                visibility: LayerVisibility::Internal,
                source: ContextSource::Template {
                    template: "priority 50".to_string(),
                },
            },
            ContextLayer {
                id: "low".to_string(),
                priority: 10,
                visibility: LayerVisibility::Internal,
                source: ContextSource::Template {
                    template: "priority 10".to_string(),
                },
            },
            ContextLayer {
                id: "mid".to_string(),
                priority: 30,
                visibility: LayerVisibility::Internal,
                source: ContextSource::Template {
                    template: "priority 30".to_string(),
                },
            },
        ];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("open", &HashMap::new());

        assert_eq!(ctx.internal_instructions.len(), 3);
        assert_eq!(ctx.internal_instructions[0], "priority 10");
        assert_eq!(ctx.internal_instructions[1], "priority 30");
        assert_eq!(ctx.internal_instructions[2], "priority 50");
    }

    #[test]
    fn test_template_variable_substitution() {
        let layers = vec![ContextLayer {
            id: "phase".to_string(),
            priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Template {
                template: "当前处于 {{ state }} 阶段，模式为 {{ mode }}".to_string(),
            },
        }];

        let mut vars = HashMap::new();
        vars.insert("mode".to_string(), "subagent".to_string());

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("design", &vars);

        assert_eq!(ctx.internal_instructions.len(), 1);
        assert_eq!(
            ctx.internal_instructions[0],
            "当前处于 design 阶段，模式为 subagent"
        );
    }

    #[test]
    fn test_conditional_layer_state_matches_included() {
        let layers = vec![ContextLayer {
            id: "design-only".to_string(),
            priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Conditional {
                condition: LayerCondition::StateMatches {
                    state: "design".to_string(),
                },
                source: Box::new(ContextSource::Template {
                    template: "Design phase instructions".to_string(),
                }),
            },
        }];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("design", &HashMap::new());

        assert_eq!(ctx.internal_instructions.len(), 1);
        assert_eq!(ctx.internal_instructions[0], "Design phase instructions");
    }

    #[test]
    fn test_conditional_layer_state_matches_skipped() {
        let layers = vec![ContextLayer {
            id: "design-only".to_string(),
            priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Conditional {
                condition: LayerCondition::StateMatches {
                    state: "design".to_string(),
                },
                source: Box::new(ContextSource::Template {
                    template: "Design phase instructions".to_string(),
                }),
            },
        }];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("build", &HashMap::new());

        assert!(ctx.internal_instructions.is_empty());
    }

    #[test]
    fn test_conditional_layer_variable_set_included() {
        let layers = vec![ContextLayer {
            id: "coordinator".to_string(),
            priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Conditional {
                condition: LayerCondition::VariableSet {
                    key: "build_mode".to_string(),
                    value: "subagent-driven".to_string(),
                },
                source: Box::new(ContextSource::Template {
                    template: "Coordinator mode active".to_string(),
                }),
            },
        }];

        let mut vars = HashMap::new();
        vars.insert("build_mode".to_string(), "subagent-driven".to_string());

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("build", &vars);

        assert_eq!(ctx.internal_instructions.len(), 1);
        assert_eq!(ctx.internal_instructions[0], "Coordinator mode active");
    }

    #[test]
    fn test_conditional_layer_variable_set_skipped() {
        let layers = vec![ContextLayer {
            id: "coordinator".to_string(),
            priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Conditional {
                condition: LayerCondition::VariableSet {
                    key: "build_mode".to_string(),
                    value: "subagent-driven".to_string(),
                },
                source: Box::new(ContextSource::Template {
                    template: "Coordinator mode active".to_string(),
                }),
            },
        }];

        let mut vars = HashMap::new();
        vars.insert("build_mode".to_string(), "direct".to_string());

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("build", &vars);

        assert!(ctx.internal_instructions.is_empty());
    }

    #[test]
    fn test_empty_layers_produces_empty_context() {
        let assembler = ContextAssembler::new(vec![]);
        let ctx = assembler.assemble("open", &HashMap::new());

        assert!(ctx.internal_instructions.is_empty());
        assert!(ctx.visible_content.is_empty());
    }

    #[test]
    fn test_mixed_visibility_and_priority_sorting() {
        let layers = vec![
            ContextLayer {
                id: "vis-high".to_string(),
                priority: 40,
                visibility: LayerVisibility::Visible,
                source: ContextSource::Template {
                    template: "visible high".to_string(),
                },
            },
            ContextLayer {
                id: "int-high".to_string(),
                priority: 30,
                visibility: LayerVisibility::Internal,
                source: ContextSource::Template {
                    template: "internal high".to_string(),
                },
            },
            ContextLayer {
                id: "vis-low".to_string(),
                priority: 10,
                visibility: LayerVisibility::Visible,
                source: ContextSource::Template {
                    template: "visible low".to_string(),
                },
            },
            ContextLayer {
                id: "int-low".to_string(),
                priority: 5,
                visibility: LayerVisibility::Internal,
                source: ContextSource::Template {
                    template: "internal low".to_string(),
                },
            },
        ];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("open", &HashMap::new());

        assert_eq!(ctx.internal_instructions.len(), 2);
        assert_eq!(ctx.internal_instructions[0], "internal low");
        assert_eq!(ctx.internal_instructions[1], "internal high");

        assert_eq!(ctx.visible_content.len(), 2);
        assert_eq!(ctx.visible_content[0], "visible low");
        assert_eq!(ctx.visible_content[1], "visible high");
    }

    #[test]
    fn test_condition_variable_set_when_key_missing() {
        let layers = vec![ContextLayer {
            id: "missing-key".to_string(),
            priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Conditional {
                condition: LayerCondition::VariableSet {
                    key: "nonexistent".to_string(),
                    value: "value".to_string(),
                },
                source: Box::new(ContextSource::Template {
                    template: "should not appear".to_string(),
                }),
            },
        }];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("open", &HashMap::new());

        assert!(ctx.internal_instructions.is_empty());
    }

    #[test]
    fn test_multiple_layers_same_priority_stable_order() {
        let layers = vec![
            ContextLayer {
                id: "first".to_string(),
                priority: 10,
                visibility: LayerVisibility::Internal,
                source: ContextSource::Template {
                    template: "first".to_string(),
                },
            },
            ContextLayer {
                id: "second".to_string(),
                priority: 10,
                visibility: LayerVisibility::Internal,
                source: ContextSource::Template {
                    template: "second".to_string(),
                },
            },
        ];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("open", &HashMap::new());

        assert_eq!(ctx.internal_instructions.len(), 2);
        // Same priority preserves insertion order
        assert_eq!(ctx.internal_instructions[0], "first");
        assert_eq!(ctx.internal_instructions[1], "second");
    }

    #[test]
    fn test_template_without_placeholders_passes_through() {
        let layers = vec![ContextLayer {
            id: "plain".to_string(),
            priority: 10,
            visibility: LayerVisibility::Visible,
            source: ContextSource::Template {
                template: "This text has no placeholders.".to_string(),
            },
        }];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("design", &HashMap::new());

        assert_eq!(ctx.visible_content.len(), 1);
        assert_eq!(ctx.visible_content[0], "This text has no placeholders.");
    }

    #[test]
    fn test_file_source_skipped() {
        let layers = vec![ContextLayer {
            id: "from-file".to_string(),
            priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::File {
                path: PathBuf::from("/nonexistent/path.md"),
            },
        }];

        let assembler = ContextAssembler::new(layers);
        let ctx = assembler.assemble("open", &HashMap::new());

        assert!(ctx.internal_instructions.is_empty());
        assert!(ctx.visible_content.is_empty());
    }

    #[test]
    fn test_serde_serialization_layer_condition_state_matches() {
        let condition = LayerCondition::StateMatches {
            state: "build".to_string(),
        };
        let json = serde_json::to_string(&condition).expect("serialize");
        // Check deserialization round-trips
        let back: LayerCondition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            back,
            LayerCondition::StateMatches {
                state: "build".to_string()
            }
        );
    }

    #[test]
    fn test_serde_serialization_full_layer() {
        let layer = ContextLayer {
            id: "test-layer".to_string(),
            priority: 42,
            visibility: LayerVisibility::Visible,
            source: ContextSource::Conditional {
                condition: LayerCondition::VariableSet {
                    key: "env".to_string(),
                    value: "prod".to_string(),
                },
                source: Box::new(ContextSource::Template {
                    template: "production mode".to_string(),
                }),
            },
        };

        let json = serde_json::to_string_pretty(&layer).expect("serialize");
        let roundtripped: ContextLayer = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(roundtripped, layer);
    }
}
