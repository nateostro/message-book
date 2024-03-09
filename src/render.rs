use chrono::{DateTime, Local};
use imessage_database::{
    tables::messages::{BubbleType, Message},
    util::dates::get_offset,
};
use regex::Regex;
use reqwest::blocking::Client;
use scraper::{Html, Selector};

// Asynchronous function to fetch the title of a webpage
fn fetch_title(url: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let client = Client::new();
    let res = client.get(url).send()?;
    let body: String = res.text()?;
    let document = Html::parse_document(&body);
    let selector = Selector::parse("title").unwrap();

    Ok(document
        .select(&selector)
        .next()
        .map(|n| n.text().collect()))
}

/// Make necessary replacements so that the text is ready for insertion
/// into latex
fn latex_escape(text: String) -> String {
    let url_regex = Regex::new(r"https?://[^\s]+").expect("Invalid URL regex");
    let urls: Vec<&str> = url_regex.find_iter(&text).map(|m| m.as_str()).collect();
    let mut url_to_title = std::collections::HashMap::new();

    // Define a list of undesired strings in titles
    let undesired_titles = vec![
        "Page Not Found",
        "Access Denied",
        "Access to this page has been denied",
        "404",
        "404 not found",
        "Update Your Browser | Facebook",
        "403 Forbidden",
        "400 Bad Request",
        "Blocked",
    ];

    for &url in &urls {
        if let Ok(Some(title)) = fetch_title(url) {
            // Check if the title contains any of the undesired strings
            if !undesired_titles
                .iter()
                .any(|&undesired| title.contains(undesired))
            {
                // Simplify the title by replacing newlines and tabs with spaces
                let simplified_title = title.replace('\n', " ").replace('\t', " ");
                url_to_title.insert(url, simplified_title);
            }
        }
    }

    let escaped = url_regex
        .replace_all(&text, |caps: &regex::Captures| {
            let url = &caps[0];
            let core_part = url.split('/').nth(2).unwrap_or(url);
            if let Some(title) = url_to_title.get(url) {
                format!("(link: {}, {})", core_part, title)
            } else {
                format!("(link: {})", core_part)
            }
        })
        .to_string();

    // Perform the rest of the replacements as before
    let escaped = escaped
        .replace("’", "'")
        .replace("“", "\"")
        .replace("”", "\"")
        .replace("…", "...")
        .replace(r"\", r"\textbackslash\ ")
        .replace("$", r"\$")
        .replace("%", r"\%")
        .replace("&", r"\&")
        .replace("_", r"\_")
        .replace("^", r"\textasciicircum\ ")
        .replace("~", r"\textasciitilde\ ")
        .replace("#", r"\#")
        .replace(r"{", r"\{")
        .replace(r"}", r"\}")
        .replace("\n", "\\newline\n")
        .replace("\u{FE0F}", "");

    // Emoji replacement
    let emoji_regex =
        Regex::new(r"(\p{Extended_Pictographic}+)").expect("Couldn't compile emoji regex");
    let demojid = emoji_regex
        .replace_all(&escaped, "{\\emojifont $1}")
        .into_owned();

    demojid
}

struct LatexMessage {
    is_from_me: bool,
    body_text: Option<String>,
    attachment_count: i32,
    date: DateTime<Local>,
}

impl LatexMessage {
    // Add a new parameter `insert_extra_space`
    fn render(self, insert_extra_space: bool) -> String {
        let mut content = match self.body_text {
            Some(ref text) => latex_escape(text.to_string()),
            None => "".to_string(),
        };

        if self.attachment_count > 0 {
            if !content.is_empty() {
                content.push_str("\\enskip")
            }
            content.push_str(
                format!(
                    "\\fbox{{{} Attachment{}}}",
                    self.attachment_count,
                    if self.attachment_count == 1 { "" } else { "s" }
                )
                .as_ref(),
            );
        }

        let date_str = self.date.format("%B %e, %Y").to_string();
        let mut rendered = format!("\\markright{{{}}}\n", date_str);

        // Insert the extra space if required
        if insert_extra_space {
            rendered.push_str("\\insertextraspace\n");
        }

        rendered.push_str(&match self.is_from_me {
            true => format!("\\leftmsg{{{}}}\n\n", content),
            false => format!("\\rightmsg{{{}}}\n\n", content),
        });

        rendered
    }
}

pub fn render_message(msg: &Message, insert_extra_space: bool) -> String {
    let parts = msg.body();

    let mut latex_msg = LatexMessage {
        is_from_me: msg.is_from_me,
        body_text: None,
        attachment_count: 0,
        date: msg
            .date(&get_offset())
            .expect("could not find date for message"),
    };

    for part in parts {
        match part {
            BubbleType::Text(text) => latex_msg.body_text = Some(text.to_owned()),
            BubbleType::Attachment => latex_msg.attachment_count += 1,
            _ => (),
        }
    }

    latex_msg.render(insert_extra_space)
}
