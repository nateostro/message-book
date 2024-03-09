use chrono::{DateTime, Local};
use imessage_database::{
    tables::messages::{BubbleType, Message},
    util::dates::get_offset,
};
use regex::Regex;

/// Make necessary replacements so that the text is ready for insertion
/// into latex
fn latex_escape(text: String) -> String {
    // Regular expression to match URLs

    // TODO: gotta be a more efficient way to do this
    let escaped = text
        // first, a bunch of weird characters replaced with ascii
        .replace("’", "'")
        .replace("“", "\"")
        .replace("”", "\"")
        .replace("…", "...")
        // now, actual latex escapes
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
        .replace("\n", "\\newline\n") // since a single newline in latex doesn't make a line break, need to add explicit newlines
        // this last one is the "variation selector" which I think determines whether an emoji
        // should be displayed big or inline. The latex font doesn't support it, so we just strip it out.
        // More info here: https://stackoverflow.com/questions/38100329/what-does-u-ufe0f-in-an-emoji-mean-is-it-the-same-if-i-delete-it
        .replace("\u{FE0F}", "");

    let url_regex = Regex::new(r"https?://[^\s]+").expect("Invalid URL regex");

    // Wrap URLs with \url{}
    let escaped = url_regex.replace_all(&escaped, r"\url{$0}").to_string();

    // Now, we wrap emojis in {\emojifont XX}. The latex template has a different font for emojis, and
    // this allows emojis to use that font
    // TODO: Somehow move this regex out so we only compile it once
    let emoji_regex =
        Regex::new(r"(\p{Extended_Pictographic}+)").expect("Couldn't compile demoji regex");
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
