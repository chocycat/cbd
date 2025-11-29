use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use x11rb::{
    connection::Connection,
    protocol::{
        Event, xfixes,
        xproto::{Atom, AtomEnum, ConnectionExt, Window, WindowClass},
    },
    rust_connection::RustConnection,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    let window = conn.generate_id()?;
    let _ = conn.create_window(
        screen.root_depth,
        window,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &Default::default(),
    );

    let clipboard_atom = get_atom(&conn, "CLIPBOARD")?;
    let targets_atom = get_atom(&conn, "TARGETS")?;
    let property_atom = get_atom(&conn, "_CLIPBOARD_DATA")?;

    xfixes::query_version(&conn, 5, 0)?;
    xfixes::select_selection_input(
        &conn,
        window,
        clipboard_atom,
        xfixes::SelectionEventMask::SET_SELECTION_OWNER,
    )?;

    let _ = conn.flush();

    loop {
        let event = conn.wait_for_event();
        if let Ok(Event::XfixesSelectionNotify(notify)) = event {
            match get_clipboard(
                &conn,
                window,
                clipboard_atom,
                targets_atom,
                property_atom,
                notify.timestamp,
            ) {
                Ok(info) => {
                    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

                    let output = json!({
                        "content": info.content,
                        "content_type": info.content_type,
                        "mime_types": info.mime_types,
                        "timestamp": timestamp,
                    });

                    println!("{}", serde_json::to_string(&output)?);
                }
                Err(e) => eprintln!("failed to get clipboard: {}", e),
            }
        }
    }
}

fn get_atom(conn: &RustConnection, name: &str) -> Result<Atom, Box<dyn std::error::Error>> {
    let reply = conn.intern_atom(false, name.as_bytes())?.reply()?;
    Ok(reply.atom)
}

struct ClipboardInfo {
    content: String,
    content_type: String,
    mime_types: Vec<String>,
}

fn get_clipboard(
    conn: &RustConnection,
    window: Window,
    clipboard: Atom,
    targets_atom: Atom,
    property: Atom,
    timestamp: u32,
) -> Result<ClipboardInfo, Box<dyn std::error::Error>> {
    conn.convert_selection(window, clipboard, targets_atom, property, timestamp)?;
    conn.flush()?;

    loop {
        let event = conn.wait_for_event()?;
        if let Event::SelectionNotify(_) = event {
            break;
        }
    }

    let reply = conn
        .get_property(false, window, property, AtomEnum::ATOM, 0, 1024)?
        .reply()?;
    let mut targets: Vec<Atom> = Vec::new();
    if let Some(value) = reply.value32() {
        targets = value.collect();
    }

    let mut mime_types = Vec::new();
    let mut target_names = Vec::new();
    for &target in &targets {
        if let Ok(name_reply) = conn.get_atom_name(target)?.reply() {
            let name = String::from_utf8_lossy(&name_reply.name).to_string();
            mime_types.push(name.clone());
            target_names.push((target, name));
        }
    }

    let (target_atom, content_type) = target_names
        .iter()
        .find(|(_, n)| {
            !matches!(
                n.as_str(),
                "TARGETS" | "TIMESTAMP" | "SAVE_TARGETS" | "MULTIPLE"
            )
        })
        .unwrap();

    conn.convert_selection(window, clipboard, *target_atom, property, timestamp)?;
    conn.flush()?;

    loop {
        let event = conn.wait_for_event()?;
        if let Event::SelectionNotify(_) = event {
            break;
        }
    }

    let reply = conn
        .get_property(true, window, property, *target_atom, 0, u32::MAX)?
        .reply()?;
    let content = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &reply.value);

    Ok(ClipboardInfo {
        content,
        content_type: content_type.to_string(),
        mime_types,
    })
}
