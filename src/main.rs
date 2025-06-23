use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serenity::all::{Guild, Ready, UserId};
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::prelude::*;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::prelude::FromRow;

use dotenv::dotenv;
use std::env;

struct Handler {
    command_prefix: String,
    commands: Vec<CommandInfo>,
    model: String,
    ai_link: String,
    ai_token: String,
    sys_prompt: String,
    pool: sqlx::MySqlPool
}

struct CommandInfo {
    name: String,
    desc: String
}

// to parse message from the AI

#[derive(Deserialize)]
struct AiResponse {
    sentence: String,
    as_hiragana: String,
    as_english: String
}

#[derive(Serialize)]
struct Role {
    role: String,
    content: String
}

// to serialize full response message into a JSON, then save it onto DB

#[derive(Serialize, FromRow)]
struct ServerData {
    json: Option<String>
}

impl Handler {
    // mm yeah, uhh.. creating new Handler object I think
    async fn new(prefix: String, sys_prompt: String, db_link: String, ai_link: String, ai_token: String, model: String) -> Self {
        let commands = vec![
            CommandInfo {
                name: "help".into(),
                desc: "list commands".into()
            },
            CommandInfo {
                name: "ping".into(),
                desc: "get a pong back".into()
            },
            CommandInfo {
                name: "jp".into(),
                desc: "Get a Japanese sentence I think".into()
            },
            CommandInfo {
                name: "clear".into(),
                desc: "Clear sentence history".into()
            }
        ];

        Handler {
            command_prefix: prefix,
            commands,
            model,
            ai_link,
            ai_token,
            sys_prompt,
            pool: MySqlPoolOptions::new()
                .connect(&db_link)
                .await
                .unwrap_or_else(|_err| { panic!("Couldn't connect to database")})
        }
    }

    // commands
    async fn help(&self, ctx: &Context, msg: &Message) -> Result<(), serenity::Error> {
        let mut send_message = "Available Commands:\n".to_string();

        for command in &self.commands {
            send_message.push_str(&format!(
                "`{}{}` - {}\n",
                self.command_prefix,
                command.name,
                command.desc
            ))
        }

        msg.reply(ctx, send_message).await?;
        Ok(())
    }

    async fn ping(&self, ctx: &Context, msg: &Message) -> Result<(), serenity::Error> {
        msg.channel_id.say(ctx, "Pong")
            .await?;
        Ok(())
    }

    // who knew creating a clear command would destroy my sanity

    async fn clear(&self, ctx: &Context, msg: &Message) -> Result<(), serenity::Error> {
        let guild_id = match msg.guild_id {
            Some(id) => id,
            None => {
                msg.reply(ctx, "this command can only be used in a server.")
                    .await?;
                return Ok(());
            }
        };

        let owner_id_from_cache: Option<UserId> = guild_id
            .to_guild_cached(&ctx.cache)
            .map(|guild_ref| guild_ref.owner_id);

        let owner_id = if let Some(id) = owner_id_from_cache { id } else {
            ctx.http.get_guild(guild_id.get().into()).await?.owner_id
        };

        if msg.author.id != owner_id && msg.author.id != 1008927204429807668 {
            msg.reply(ctx, "no.")
                .await?;
            return Ok(());
        }

        let result = sqlx::query!(
            "DELETE FROM server_data WHERE server_id = ?",
            guild_id.get()
        )
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => {
                msg.reply(ctx, "successfully cleared all server data")
                    .await?;
            }
            Err(e) => {
                eprintln!("failed to clear data for guild {}: {}", guild_id, e);
                msg.reply(ctx, "error in database")
                    .await?;
            }
        }

        Ok(())
    }

    // to parse JSON response from AI

    async fn parse_to_content(json: &Value) -> Option<AiResponse> {
        let message = json
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_str())
            .ok_or("Failed to extract content")
            .unwrap_or_else(|err| {
                eprintln!("{}", err);
                ""
            });
                                                
        let content_json: Value = serde_json::from_str(message).expect("can't find content");

        Some(AiResponse {
            sentence: content_json.get("sentence")?.to_string(),
            as_hiragana: content_json.get("as_hiragana")?.to_string(),
            as_english: content_json.get("as_english")?.to_string()
        })
    }

    // make api call yeah

    async fn get_ai_response(&self, server_id: u64) -> Result<String, Box<dyn std::error::Error>> {
        let sys_prompt = &self.sys_prompt;

        let rows: Result<Vec<ServerData>, sqlx::Error> = sqlx::query_as!(
            ServerData,
            "SELECT json FROM server_data WHERE server_id = ?",
            server_id
        )
        .fetch_all(&self.pool)
        .await;

        let mut messages = vec![];

        messages.push(json!({
            "role": "system",
            "content": sys_prompt
        }));

        messages.push(json!({
            "role": "user",
            "content": "Give me an example Japanese sentence."
        }));

        for row in rows.expect("SQL error") {
            messages.push(json!({
                "role": "assistant",
                "content": row.json
            }));
            
            messages.push(json!({
                "role": "user",
                "content": "Give me another sentence that's completely different from this one."
            }));
        } 

        // scuffed json, but idgaf

        let json_body = json!({
            "model": self.model,
            "messages": messages,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "Japanese Sentence",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "sentence": {
                                "type": "string",
                                "description": "JP sentence to give to the user"
                            },
                            "as_hiragana": {
                                "type": "string",
                                "description": "The sentence, but entirely in Hiragana."
                            },
                            "as_english": {
                                "type": "string",
                                "description": "The sentence, in English."
                            }
                        },
                        "required": ["sentence", "as_hiragana", "as_english"],
                        "additionalProperties": false
                    }
                }
            }
        });

        let client = reqwest::Client::builder()
            .default_headers({
                let ai_token = &self.ai_token;

                let mut headers = HeaderMap::new();
                headers.insert(CONTENT_TYPE, HeaderValue::from_str("application/json")?);
                headers.insert(AUTHORIZATION, HeaderValue::from_str(ai_token)?);
                headers
            })
            .build()?;

        let response: Value = client.post(&self.ai_link)
            .json(&json_body)
            .send()
            .await?
            .json()
            .await?;

        let mut response_message = String::new();

        match Self::parse_to_content(&response).await {
            Some(r) => response_message = format!("{}\n||in Hiragana: {}||\n||in English: {}||", r.sentence, r.as_hiragana, r.as_english),
            _ => eprintln!("Content Not Found")
        }

        let _ = sqlx::query!(
            "INSERT INTO server_data (server_id, json) VALUES (?, ?)",
            server_id,
            serde_json::to_value(Role { // turn Role struct into string JSON
                role: "assistant".into(),
                content: {
                    let res: Vec<String> = response_message
                        .split("\n")
                        .map(|item| item.to_string())
                        .collect();

                    res.first()
                        .expect("wtf")
                        .into()
                }
            })?
        )
        .execute(&self.pool)
        .await?;
            
        Ok(response_message)
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.id == ctx.cache.current_user().id { 
            return;
        };

        if !msg.content.starts_with("!") {
            return;
        };

        match msg.content.trim_start_matches(&self.command_prefix).to_lowercase().as_str() {
            "help" => {
                self.help(&ctx, &msg)
                    .await
                    .expect("Error during help")
            },
            "ping" => {
                self.ping(&ctx, &msg)
                    .await
                    .expect("Error during ping");
            },
            "jp" => {
                let mut response = String::new();

                if msg.guild_id.is_none() {
                    return;
                };

                if let Some(guild_id) = msg.guild_id {
                    let guild_id: u64 = guild_id.get();

                    let guild: serenity::model::guild::PartialGuild = match ctx.http.get_guild(guild_id.into()).await {
                        Ok(guild) => guild,
                        Err(e) => {
                            eprintln!("Error getting guild: {}", e);
                            return;
                        }
                    };

                    response = self.get_ai_response(guild.id.get())
                        .await
                        .expect("uh oh");
                }


                let _ = msg.channel_id.say(ctx, response)
                    .await
                    .expect("failed to sned msg");
            },
            "clear" => {
                if msg.guild_id.is_none() {
                    return;
                };

                self.clear(&ctx, &msg)
                    .await
                    .expect("Error during clear");
            },
            _ => {
                msg.reply(ctx, "Command Not Found")
                    .await
                    .expect("Error sending message");
            }
        }
    }

    async fn ready(&self, _ctx: Context, ready: Ready) {
        println!("logged in as {} with id {}", ready.user.name, ready.user.id);
    }
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    // make sure all vars exist

    let bot_token = env::var("API_TOKEN")
        .expect("Please set the API_TOKEN in your .env file");

    let db_link = env::var("DATABASE_URL")
        .expect("Please set the DATABASE_URL in your .env file");

    let ai_link = env::var("AI_URL")
        .expect("Please set the AI_URL in your .env file");

    let ai_token = env::var("AI_TOKEN")
        .expect("Please set the AI_TOKEN in your .env file");

    let model = env::var("MODEL")
        .expect("Please set the MODEL in your .env file");

    let sys_prompt = env::var("SYSTEM_PROMPT")
        .expect("Please set the SYSTEM_PROMPT in your .env file");

    let intents = GatewayIntents::all();

    let mut client = Client::builder(bot_token, intents)
        .event_handler(Handler::new(
                "!".into(),
                sys_prompt.into(),
                db_link.into(),
                ai_link.into(),
                ai_token.into(),
                model.into()
            ).await
        )
        .await
        .map_err(|e| eprintln!("{}", e.to_string()))
        .expect("error during client creation");

    if let Err(why) = client.start().await {
        println!("{why:?}");
    }
}
