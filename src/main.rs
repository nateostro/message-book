use anyhow::Result;
use clap::Parser;
use dirs::home_dir;
use imessage_database::{
    error::table::TableError,
    tables::{
        chat::Chat,
        messages::Message,
        table::{
            get_connection, Table, CHAT_MESSAGE_JOIN, DEFAULT_PATH_IOS, DEFAULT_PATH_MACOS,
            MESSAGE, MESSAGE_ATTACHMENT_JOIN, RECENTLY_DELETED,
        },
    },
    util::dates::get_offset,
};
use render::render_message;
use rusqlite::types::Value;
use std::{
    fs::{copy, create_dir_all, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
};

mod render;

const TEMPLATE_DIR: &str = "templates";

// default ios sms.db path is <backup-path>/3d/3d0d7e5fb2ce288813306e4d4636395e047a3d2

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Phone number of the conversation to export, of the form '+15555555555'
    recipient: String,
    /// Where to find the iMessage database. If not provided, assumes it is in the default Mac location
    #[clap(flatten)]
    database_location: BackupPath,
    /// The directory to create the .tex files in
    #[arg(short, long, default_value = "output")]
    output_dir: PathBuf,
}

impl Args {
    fn get_db_location(&self) -> PathBuf {
        match &self.database_location {
            BackupPath {
                ios_backup_dir: Some(ios_dir),
                chat_database: None,
            } => {
                let mut path = ios_dir.clone();
                path.push(DEFAULT_PATH_IOS);
                path
            }
            BackupPath {
                ios_backup_dir: None,
                chat_database: Some(db_path),
            } => db_path.to_owned(),
            BackupPath {
                ios_backup_dir: None,
                chat_database: None,
            } => {
                let home_directory = home_dir().expect("Could not find home directory");
                home_directory.join(&DEFAULT_PATH_MACOS)
            }
            _ => panic!("too many arguments for database location"),
        }
    }
}

#[derive(Debug, clap::Args)]
#[group(required = false, multiple = false)]
struct BackupPath {
    /// Path to the root of an iOS backup folder
    #[arg(short, long)]
    ios_backup_dir: Option<PathBuf>,
    /// Path to the chat database directly. If neither `ios_backup_dir` or `chat_database` is provided, the location will be assumed to be the default MacOS location.
    #[arg(short, long)]
    chat_database: Option<PathBuf>,
}

fn iter_messages(
    db_path: &PathBuf,
    chat_identifier: &str,
    output_dir: &PathBuf,
) -> Result<(), TableError> {
    println!("Inside iter_messages, using db path: {:?}", db_path);
    let db = get_connection(db_path).unwrap();

    let mut chat_stmt = Chat::get(&db)?;
    let chats: Vec<Chat> = chat_stmt
        .query_map([], |row| Chat::from_row(row))
        .unwrap()
        .filter_map(|c| c.ok())
        .filter(|c| c.chat_identifier == chat_identifier)
        .collect(); // we collect these into a vec since there should only be a couple, we don't need to stream them

    let chat_ids: Vec<i32> = chats.iter().map(|c| c.rowid).collect();

    // let mut msg_stmt = Message::get(&db)?;
    // using rarray as in the example at https://docs.rs/rusqlite/0.29.0/rusqlite/vtab/array/index.html to check if chat is ok
    // SQL almost entirely taken from imessage-database Message::get, with added filtering
    rusqlite::vtab::array::load_module(&db).expect("failed to load module");
    let mut msg_stmt = db.prepare(&format!(
        "SELECT
                 *,
                 c.chat_id,
                 (SELECT COUNT(*) FROM {MESSAGE_ATTACHMENT_JOIN} a WHERE m.ROWID = a.message_id) as num_attachments,
                 (SELECT b.chat_id FROM {RECENTLY_DELETED} b WHERE m.ROWID = b.message_id) as deleted_from,
                 (SELECT COUNT(*) FROM {MESSAGE} m2 WHERE m2.thread_originator_guid = m.guid) as num_replies
             FROM
                 message as m
                 LEFT JOIN {CHAT_MESSAGE_JOIN} as c ON m.ROWID = c.message_id
             WHERE
                 c.chat_id IN rarray(?1)
             ORDER BY
                 m.date
             LIMIT
                 100000;
            "
        )).expect("unable to build messages query");

    // unfortunately I don't think there is an easy way to add a WHERE clause
    // to the statement generated by Message::get.
    // So instead I generated my own SQL statement, based on Message::get
    // and I need to pass in the valid chat ids
    let chat_id_values = Rc::new(
        chat_ids
            .iter()
            .copied()
            .map(Value::from)
            .collect::<Vec<Value>>(),
    );
    let msgs = msg_stmt
        .query_map([chat_id_values], |row| Message::from_row(row))
        .unwrap()
        .filter_map(|m| m.ok());
    // .filter(|m| m.chat_id.is_some_and(|id| chat_ids.contains(&id))); // not needed with new sql filtering

    chats.iter().for_each(|c| println!("Found chat {:?}", c));

    // need to create output dir first, so we can create files inside it
    create_dir_all(output_dir).expect("Could not create output directory");

    let filtered_msgs =
        msgs.filter(|m| !m.is_reaction() && !m.is_announcement() && !m.is_shareplay());

    let mut chapters: Vec<String> = vec![];
    let mut current_output_info: Option<(String, File)> = None;
    let mut last_message_side: Option<bool> = None; // Track the side of the last message

    for mut msg in filtered_msgs {
        let msg_date = msg
            .date(&get_offset())
            .expect("could not find date for message");
        let chapter_name = msg_date.format("ch-%Y-%m").to_string();
        let out_fname = format!("{}.tex", &chapter_name);

        // Determine if a new chapter file needs to be created
        let create = match &current_output_info {
            None => true,
            Some((ref name, _)) => name != &chapter_name,
        };

        if create {
            let out_path = Path::join(output_dir, &out_fname);
            let mut f = File::create(&out_path).unwrap_or_else(|e| {
                panic!(
                    "Failed to create output file: {} - {:?}",
                    &out_path.to_string_lossy(),
                    e
                )
            });
            f.write(
                format!("\\chapter{{{}}}\n\n", msg_date.format("%B %Y").to_string()).as_bytes(),
            )
            .expect("Could not write to chapter file");
            current_output_info = Some((chapter_name.clone(), f));
            chapters.push(chapter_name);
            last_message_side = None; // Reset for each new chapter
        }

        // Determine if \insertextraspace should be inserted
        let insert_extra_space = last_message_side == Some(msg.is_from_me);
        last_message_side = Some(msg.is_from_me); // Update last message side

        match msg.gen_text(&db) {
            Ok(_) => {
                // Adjust the call to render_message to pass the new parameter
                let rendered = render_message(&msg, insert_extra_space); // Adjusted to pass insert_extra_space
                let mut output_file = &current_output_info
                    .as_ref()
                    .expect("Current output info was none while processing message")
                    .1;
                output_file
                    .write(rendered.as_bytes())
                    .expect("Unable to write message to output file");
            }
            Err(err) => {
                eprintln!("Failed to generate message: {:?}", err);
            }
        }
    }

    // Once we create all the chapter files, we need to create the main.tex file to include them
    let mut main_template_file = File::open(
        [TEMPLATE_DIR, "main.tex.template"]
            .iter()
            .collect::<PathBuf>(),
    )
    .expect("Could not open template file");
    let mut main_template = String::new();
    main_template_file
        .read_to_string(&mut main_template)
        .expect("could not read template main.tex");
    let mut main_tex_file = File::create(Path::join(output_dir, "main.tex"))
        .expect("could not create main.tex in output dir");
    main_tex_file
        .write_all(main_template.as_bytes())
        .expect("Could not write main.tex");

    // now add the chapters to the main file
    chapters.iter().for_each(|chapter_name| {
        main_tex_file
            .write(format!("\\include{{{}}}\n", chapter_name).as_bytes())
            .expect("failed to write main file");
    });

    // Copy the emoji font file to the output folder
    let emoji_font_source = Path::new("tex/NotoEmoji-Medium.ttf");
    let emoji_font_destination = output_dir.join("NotoEmoji-Medium.ttf");
    std::fs::copy(emoji_font_source, &emoji_font_destination)
        .expect("Could not copy emoji font file");

    // and finish it with \end{document}
    // TODO: we should really do this with a templating engine
    main_tex_file
        .write(r"\end{document}".as_bytes())
        .expect("unable to finish main.tex");

    // finally, copy over the makefile
    copy(
        [TEMPLATE_DIR, "Makefile"].iter().collect::<PathBuf>(),
        Path::join(output_dir, "Makefile"),
    )
    .expect("Could not copy makefile");

    Ok(())
}

fn main() {
    let args = Args::parse();
    let db_path = args.get_db_location();
    iter_messages(&db_path, &args.recipient, &args.output_dir).expect("failed :(");

    println!("Hello!")
}
