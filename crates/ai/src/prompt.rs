//! Builds the request for each thing the user can ask the assistant to do.

use serde::Deserialize;

use crate::context::SchemaContext;
use crate::{ChatMessage, ChatRequest, Role};

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Task {
    /// Explain what the statement does.
    Explain,
    /// Check the statement for mistakes and propose a corrected one.
    Review,
    /// Propose a faster statement, and indexes where they would help.
    Optimize,
    /// A free question, about the statement if there is one. Also a chat turn.
    Ask { question: String },
}

pub struct Database<'a> {
    /// Engine name as shown to the user, e.g. "PostgreSQL".
    pub engine: &'a str,
    pub version: Option<&'a str>,
    pub schema: &'a SchemaContext,
}

/// The conventions the interface relies on to offer "Apply" and to show a verdict.
const INSTRUCTIONS: &str = "\
You are the assistant inside a desktop database client. A developer is writing SQL against the database described below and asks you for help.

What you can see: the database's structure (tables, columns, types, keys, indexes, and the database's own row-count estimates) and the SQL the user shows you. You cannot see any rows, and you cannot run anything. Never say or imply that you ran a statement or looked at data. When an answer depends on something you cannot see, such as how values are distributed or what a query plan says, say so and give the statement the user can run to find out.

Write SQL for the exact engine and version named below, using only tables and columns that appear in the structure. If the request needs a table or column that is not there, say that it is missing rather than inventing one. If the request is ambiguous in a way that changes the query, for example several date columns could be meant, ask one short question before writing SQL: write the line \"Question: \" and the question, then each likely answer on its own line as \"- Option: <value> — <a few words on what it means>\", at most four, and write no SQL in that answer.

Format:
- Put every SQL statement in a fenced block tagged sql.
- When you propose a statement meant to replace or become the user's query, write the line \"Suggested query:\" and then exactly one sql block with the complete statement, ready to run. Do not split it and do not abbreviate with comments such as \"-- rest unchanged\".
- When an index would help, write the line \"Suggested index:\" and then a sql block with the CREATE INDEX statement, and say in one sentence what it speeds up. Mention that building an index on a large table takes time and can slow writes while it runs. Suggest an index only when the structure shows that no existing index already serves the query.
- When you are asked to check a statement, begin with one line, exactly \"Verdict: OK\", \"Verdict: Warnings\" or \"Verdict: Errors\". Errors are things that make the statement fail or return wrong results. Warnings are things that work but are risky or slow, such as an UPDATE or DELETE without WHERE, SELECT * in application code, or a missing LIMIT on a large table. Then list each finding on its own line as \"- ERROR line N: …\", \"- WARNING line N: …\" or \"- NOTE line N: …\", where N is the line within the statement as it was shown to you (its first line is 1), and put the table, column or keyword the finding is about in backticks.

Statements that change or delete data deserve a plain warning about what they will affect. Keep explanations short and concrete; the reader is a developer. Answer in the language the user writes in; SQL keywords and identifiers stay as they are.";

/// Assembles the request. `history` holds earlier turns of the same conversation.
pub fn build(
    database: &Database,
    task: &Task,
    sql: Option<&str>,
    history: &[ChatMessage],
) -> ChatRequest {
    let engine = match database.version {
        Some(version) => format!("{} {version}", database.engine),
        None => database.engine.to_owned(),
    };
    let scope = match &database.schema.namespace {
        Some(namespace) => format!(", schema or database \"{namespace}\""),
        None => String::new(),
    };
    let omitted = database.schema.tables_total - database.schema.tables_detailed;
    let coverage = if omitted > 0 {
        format!(
            "\nThe structure is large: {} of {} tables are described in full and the rest are listed by name. Ask for a table's columns if you need them.",
            database.schema.tables_detailed, database.schema.tables_total
        )
    } else {
        String::new()
    };
    // Instructions first and the structure after them: the instructions never
    // change, so providers that cache a request's beginning can reuse it.
    let system = format!(
        "{INSTRUCTIONS}\n\nEngine: {engine}{scope}{coverage}\n\nStructure:\n{}",
        database.schema.text
    );

    let statement = sql.map(str::trim).filter(|s| !s.is_empty());
    let fenced = statement.map(|s| format!("```sql\n{s}\n```"));
    let content = match (task, fenced) {
        (Task::Explain, Some(sql)) => {
            format!("Explain what this statement does, step by step, and what it returns.\n\n{sql}")
        }
        (Task::Review, Some(sql)) => format!(
            "Check this statement for mistakes. If it needs changes, give the corrected statement.\n\n{sql}"
        ),
        (Task::Optimize, Some(sql)) => format!(
            "Make this statement faster without changing what it returns. Say which change matters most, and suggest an index if one would help.\n\n{sql}"
        ),
        (Task::Ask { question }, Some(sql)) => format!(
            "{}\n\nThe statement I am working on:\n\n{sql}",
            question.trim()
        ),
        (Task::Ask { question }, None) => question.trim().to_owned(),
        // The interface offers these only with a statement; without one, say so instead of guessing.
        (Task::Explain | Task::Review | Task::Optimize, None) => {
            "There is no statement in the editor yet. Tell me, in one sentence, that you need one."
                .to_owned()
        }
    };

    let mut messages = history.to_vec();
    messages.push(ChatMessage {
        role: Role::User,
        content,
    });
    ChatRequest { system, messages }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(detailed: usize, total: usize) -> SchemaContext {
        SchemaContext {
            text: "TABLE public.orders (about 8400000 rows)\n  id bigint PRIMARY KEY NOT NULL\n"
                .into(),
            namespace: Some("public".into()),
            tables_total: total,
            tables_detailed: detailed,
        }
    }

    #[test]
    fn the_request_carries_engine_structure_and_statement() {
        let schema = schema(1, 1);
        let database = Database {
            engine: "PostgreSQL",
            version: Some("17.2"),
            schema: &schema,
        };
        let request = build(
            &database,
            &Task::Review,
            Some("  SELECT * FROM orders  "),
            &[],
        );

        assert!(
            request
                .system
                .starts_with("You are the assistant inside a desktop database client.")
        );
        assert!(
            request
                .system
                .contains("Engine: PostgreSQL 17.2, schema or database \"public\"")
        );
        assert!(
            request
                .system
                .contains("TABLE public.orders (about 8400000 rows)")
        );
        assert!(!request.system.contains("The structure is large"));
        assert_eq!(request.messages.len(), 1);
        assert!(
            request.messages[0]
                .content
                .ends_with("```sql\nSELECT * FROM orders\n```")
        );
    }

    #[test]
    fn a_question_keeps_the_conversation_and_says_when_the_structure_is_partial() {
        let schema = schema(60, 140);
        let database = Database {
            engine: "MySQL",
            version: None,
            schema: &schema,
        };
        let history = vec![
            ChatMessage {
                role: Role::User,
                content: "Which table has payments?".into(),
            },
            ChatMessage {
                role: Role::Assistant,
                content: "payments".into(),
            },
        ];
        let task = Task::Ask {
            question: " Total per month? ".into(),
        };
        let request = build(&database, &task, None, &history);

        assert!(
            request
                .system
                .contains("60 of 140 tables are described in full")
        );
        assert_eq!(request.messages.len(), 3);
        assert_eq!(request.messages[2].content, "Total per month?");
        assert_eq!(request.messages[2].role, Role::User);
    }

    #[test]
    fn tasks_come_from_the_interface_as_tagged_json() {
        let task: Task = serde_json::from_str(r#"{"kind":"ask","question":"why?"}"#).unwrap();
        assert_eq!(
            task,
            Task::Ask {
                question: "why?".into()
            }
        );
        assert_eq!(
            serde_json::from_str::<Task>(r#"{"kind":"optimize"}"#).unwrap(),
            Task::Optimize
        );
    }
}
