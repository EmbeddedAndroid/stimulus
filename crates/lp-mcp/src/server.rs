use lp_core::ops::Dispatcher;
use serde_json::Value;
use std::io::{self, BufRead, Write};

pub fn run_stdio(
    dispatcher: &dyn Dispatcher,
    input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    let server = crate::McpServer::default();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => server.handle(dispatcher, request),
            Err(error) => crate::protocol::failure(
                Value::Null,
                -32700,
                "parse error",
                Some(serde_json::json!({"detail":error.to_string()})),
            ),
        };
        if !response.is_null() {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lp_core::ops::OpSpec;
    use serde_json::json;
    use std::io::Cursor;

    struct Echo;
    impl Dispatcher for Echo {
        fn call(&self, op: &OpSpec, params: Value) -> Result<Value, lp_core::ToolError> {
            Ok(json!({"op":op.id,"params":params}))
        }
    }

    #[test]
    fn stdio_is_one_json_rpc_message_per_line_and_survives_parse_errors() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n",
            "not-json\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"capture_summary\",\"arguments\":{}}}\n"
        );
        let mut output = Vec::new();
        run_stdio(&Echo, Cursor::new(input), &mut output).unwrap_or_else(|error| panic!("{error}"));
        let messages = String::from_utf8(output)
            .unwrap_or_else(|error| panic!("{error}"))
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap_or_else(|error| panic!("{error}"))
            })
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["result"], json!({}));
        assert_eq!(messages[1]["error"]["code"], -32700);
        assert_eq!(
            messages[2]["result"]["structuredContent"]["op"],
            "capture.summary"
        );
    }
}
