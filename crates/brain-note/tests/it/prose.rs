use brain_note::prose;

/// Whether a line saying `seen` is read as text rather than code.
fn read(body: &str) -> bool {
    prose(body).any(|line| line.contains("seen"))
}

#[test]
fn what_is_inside_a_block_is_not_read() {
    for body in [
        "1. ```sh\n   # seen\n   ```",
        // A block a little way in may hold lines further left than its marks.
        "  ```\n# seen\n  ```",
        // A blank line inside a nested block does not end it.
        "10. ```\n    a\n\n    seen\n    ```",
    ] {
        assert!(!read(body), "{body:?}");
    }
}

#[test]
fn what_opens_no_block_hides_nothing() {
    for body in [
        "~5 minutes\nseen",
        "~~gone~~\nseen",
        "`brain due`\nseen",
        // Tabs are four columns, so a bullet two tabs deep holds its tab-indented code.
        "\t\t- ```\n\t\t\tls\n\t\t\t```\n\nseen",
        "\t1. ```\n\t\tls\n\t\t```\n\nseen",
    ] {
        assert!(read(body), "{body:?}");
    }
}
