pub struct AgentDefinition {
    pub name: &'static str,
    pub system_prompt: &'static str,
    pub tools: Option<Vec<&'static str>>,
    pub spawns: Option<Vec<&'static str>>,
    pub blocking: bool,
}

pub fn builtin_agents() -> Vec<AgentDefinition> {
    vec![
        AgentDefinition {
            name: "explore",
            system_prompt: "You are a code exploration agent. Search the codebase to find \
                relevant files, functions, and patterns. Report your findings accurately.",
            tools: None,
            spawns: None,
            blocking: true,
        },
        AgentDefinition {
            name: "plan",
            system_prompt: "You are a planning agent. Analyze the task and create a \
                structured implementation plan with clear steps.",
            tools: None,
            spawns: Some(vec!["explore"]),
            blocking: true,
        },
        AgentDefinition {
            name: "reviewer",
            system_prompt: "You are a code review agent. Review the code changes for \
                correctness, style, and potential issues.",
            tools: None,
            spawns: None,
            blocking: true,
        },
        AgentDefinition {
            name: "task",
            system_prompt: "You are a general-purpose task agent. Complete the assigned \
                task thoroughly and report your results.",
            tools: None,
            spawns: Some(vec!["explore"]),
            blocking: false,
        },
        AgentDefinition {
            name: "designer",
            system_prompt: "You are a design agent. Create designs, mockups, and UI \
                specifications based on the requirements.",
            tools: None,
            spawns: None,
            blocking: true,
        },
        AgentDefinition {
            name: "oracle",
            system_prompt: "You are a knowledge agent. Answer questions about the codebase, \
                technologies, and best practices.",
            tools: None,
            spawns: Some(vec!["explore"]),
            blocking: true,
        },
        AgentDefinition {
            name: "librarian",
            system_prompt: "You are a documentation agent. Find and organize documentation, \
                API references, and usage examples.",
            tools: None,
            spawns: None,
            blocking: true,
        },
    ]
}
