use anyhow::{anyhow, Result};
use bk_tree::{BKTree, Metric};
use futures::StreamExt;
use img_hash::{Hasher, HasherConfig, ImageHash};
use std::collections::HashMap;
use std::env;
use telegram_bot::*;

#[tokio::main]
async fn main() -> Result<()> {
    let token = env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set");
    let api = Api::new(&token);
    let hasher = HasherConfig::new().preproc_dct().to_hasher();
    let mut img_db = ImageDatabase::new();

    let mut stream = api.stream();
    while let Some(update) = stream.next().await {
        if let Err(e) = handle_update(update?, &api, &token, &hasher, &mut img_db).await {
            eprintln!("{}", e);
        }
    }
    Ok(())
}

async fn handle_update(
    update: Update,
    api: &Api,
    token: &str,
    hasher: &Hasher,
    img_db: &mut ImageDatabase,
) -> Result<()> {
    if let UpdateKind::Message(message) = update.kind {
        if let MessageKind::Photo {
            ref data,
            ref caption,
            ..
        } = message.kind
        {
            let largest_photo =
                data.iter().fold(
                    &data[0],
                    |ps1, ps2| if ps1.height < ps2.height { ps2 } else { ps1 },
                );
            let photo_file_response = api.send(largest_photo.get_file()).await?;
            let file_url = format!(
                "https://api.telegram.org/file/bot{}/{}",
                &token,
                photo_file_response
                    .file_path
                    .ok_or(anyhow!("Empty file url in Telegram response"))?
            );
            let file_content = reqwest::get(&file_url).await?.bytes().await?;
            let img = image::load_from_memory(&file_content)?;
            let hash = hasher.hash_image(&img);
            if let Some(cap) = caption {
                if cap == "!!hash" {
                    api.send(message.text_reply(format!("{:?}", hash.as_bytes())))
                        .await?;
                }
            } else if img_db.exists(message.chat.id(), &hash) {
                api.send(SeenItBefore::reply_to(message.chat.id(), message.id))
                    .await?;
            } else {
                img_db.add(message.chat.id(), hash);
            }
        }
    }
    Ok(())
}

struct Distance;

impl Metric<ImageHash> for Distance {
    fn distance(&self, a: &ImageHash, b: &ImageHash) -> u64 {
        a.dist(b) as u64
    }
}

struct ImageDatabase {
    m: HashMap<ChatId, BKTree<ImageHash, Distance>>,
}

impl ImageDatabase {
    fn new() -> ImageDatabase {
        ImageDatabase { m: HashMap::new() }
    }

    fn exists(&self, cid: ChatId, h: &ImageHash) -> bool {
        if let Some(bkt) = self.m.get(&cid) {
            if let Some(_) = bkt.find(h, 5).next() {
                return true;
            }
        }
        false
    }

    fn add(&mut self, cid: ChatId, h: ImageHash) {
        self.m
            .entry(cid)
            .or_insert_with(|| BKTree::new(Distance))
            .add(h);
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
struct SeenItBefore {
    chat_id: ChatId,
    reply_to_message_id: MessageId,
}

impl SeenItBefore {
    fn reply_to(chat_id: ChatId, reply_to_message_id: MessageId) -> SeenItBefore {
        SeenItBefore {
            chat_id,
            reply_to_message_id,
        }
    }
}

impl ToMultipart for SeenItBefore {
    fn to_multipart(&self) -> Result<Multipart, telegram_bot_raw::requests::_base::Error> {
        Ok(vec![
            (
                "chat_id",
                telegram_bot_raw::requests::_base::MultipartValue::Text(
                    self.chat_id.to_string().into(),
                ),
            ),
            (
                "sticker",
                telegram_bot_raw::requests::_base::MultipartValue::Text(
                    "CAACAgUAAxkBAAMyX0Sjn0AB9RHDl1Y62MljVR2F_HkAAgYAAwfDqAvcvSc9SDpa3hsE".into(),
                ),
            ),
            (
                "reply_to_message_id",
                telegram_bot_raw::requests::_base::MultipartValue::Text(
                    self.reply_to_message_id.to_string().into(),
                ),
            ),
        ])
    }
}

impl Request for SeenItBefore {
    type Type = MultipartRequestType<Self>;
    type Response = JsonIdResponse<Message>;

    fn serialize(&self) -> Result<HttpRequest, telegram_bot_raw::requests::_base::Error> {
        Self::Type::serialize(RequestUrl::method("sendSticker"), self)
    }
}
