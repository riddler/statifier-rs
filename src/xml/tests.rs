use super::cursor::Cursor;
use super::{Attribute, Element, Location, Node, ParseError, Reason, parse};

const SCXML: &str = "http://www.w3.org/2005/07/scxml";

fn read(source: &str) -> Result<Element, ParseError> {
    parse(source.as_bytes())
}

// The root element of `source`, failing the test when it is refused.
fn root(source: &str) -> Element {
    match read(source) {
        Ok(root) => root,
        Err(error) => panic!("expected {source:?} to parse, got {}", error.message),
    }
}

// The refusal of `source`, failing the test when it parses.
fn refusal(source: &[u8]) -> ParseError {
    match parse(source) {
        Ok(_) => panic!("expected {source:?} to be refused"),
        Err(error) => error,
    }
}

// Asserts that `source` is refused for `reason` at `line` and `column`, with
// a zero-width span and a message that names the position.
fn assert_refused(source: &str, reason: Reason, line: usize, column: usize) {
    assert_refused_bytes(source.as_bytes(), reason, line, column);
}

fn assert_refused_bytes(source: &[u8], reason: Reason, line: usize, column: usize) {
    let error = refusal(source);
    assert_eq!(
        (
            error.reason,
            error.location.start_line,
            error.location.start_column
        ),
        (reason, line, column),
        "{source:?}: {}",
        error.message
    );
    assert_eq!(error.location.start_offset, error.location.end_offset);
    assert_eq!(
        (error.location.start_line, error.location.start_column),
        (error.location.end_line, error.location.end_column)
    );
    let position = format!("(line {line}, column {column})");
    assert!(error.message.ends_with(&position), "{}", error.message);
}

fn element(node: &Node) -> &Element {
    match node {
        Node::Element(element) => element,
        Node::Text(text) => panic!("expected an element, got text {:?}", text.value),
    }
}

fn text(node: &Node) -> &str {
    match node {
        Node::Text(text) => &text.value,
        Node::Element(element) => panic!("expected text, got <{}>", element.name),
    }
}

fn location(node: &Node) -> Location {
    match node {
        Node::Text(text) => text.location,
        Node::Element(element) => element.location,
    }
}

fn text_value(source: &str) -> String {
    let root = root(source);
    let first = root.children.first().expect("a child");
    text(first).to_owned()
}

fn first_attribute(source: &str) -> Attribute {
    root(source)
        .attributes
        .first()
        .expect("an attribute")
        .clone()
}

fn attribute_value(source: &str) -> String {
    first_attribute(source).value
}

fn span_of(location: Location) -> [usize; 6] {
    [
        location.start_line,
        location.start_column,
        location.start_offset,
        location.end_line,
        location.end_column,
        location.end_offset,
    ]
}

// ---- constructs

// Sabotage: with read_cdata expanding references as character data does, this test fails.
#[test]
fn takes_a_cdata_section_verbatim_and_coalesces_it_with_its_neighbours() {
    let source = "<a>a<![CDATA[<b> &amp; ]]>c</a>";
    let a = root(source);
    assert_eq!(a.children.len(), 1);
    assert_eq!(text(&a.children[0]), "a<b> &amp; c");
    assert_eq!(
        location(&a.children[0]).slice(source),
        Some("a<![CDATA[<b> &amp; ]]>c")
    );
}

// Sabotage: with step dropping the run a CDATA section was read into, this test fails.
#[test]
fn makes_a_text_node_from_a_cdata_section_alone_even_an_empty_one() {
    assert_eq!(text_value("<a><![CDATA[x]]></a>"), "x");
    assert_eq!(text_value("<a><![CDATA[]]></a>"), "");
}

// Sabotage: with step flushing the run before a comment, this test fails.
#[test]
fn coalesces_text_across_a_comment_and_a_processing_instruction() {
    let a = root("<a>x &amp;<!-- note -->y<?pi data?>z</a>");
    assert_eq!(a.children.len(), 1);
    assert_eq!(text(&a.children[0]), "x &yz");
}

// Sabotage: with step opening a run before skipping a comment, this test fails.
#[test]
fn makes_no_node_for_a_comment_or_processing_instruction_alone() {
    assert!(root("<a><!-- only --><?pi x?></a>").children.is_empty());
}

// Sabotage: with the doctype branch of skip_misc never taken, this test fails.
#[test]
fn skips_the_declaration_comments_a_doctype_and_trailing_misc() {
    let source = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- l -->\n\
                  <!DOCTYPE a [ <!ELEMENT a ANY> ]>\n<?pi x?>\n<a xmlns=\"urn:x\"/>\n\
                  <!-- after -->\n<?pi y?>\n";
    assert_eq!(
        root(source).location.slice(source),
        Some("<a xmlns=\"urn:x\"/>")
    );
}

// Sabotage: with skip_doctype moving to the bracket end, this test fails.
#[test]
fn ends_a_doctype_where_the_reference_does() {
    let a = root("<!DOCTYPE a [<!-- ] -->]><a/>");
    assert_eq!((a.location.start_line, a.location.start_column), (1, 26));
    assert_eq!(root("<!DOCTYPE a [<!-- \" -->\"]><a/>").name, "a");
    let s = root("<!DOCTYPE s [<!-- it's -->]>\n<s/>");
    assert_eq!((s.location.start_line, s.location.start_column), (2, 1));
}

// Sabotage: with flush dropping a whitespace-only run, this test fails.
#[test]
fn keeps_whitespace_only_text_runs() {
    let a = root("<a>\n  <b/>\n</a>");
    let kinds: Vec<bool> = a
        .children
        .iter()
        .map(|node| matches!(node, Node::Text(_)))
        .collect();
    assert_eq!(kinds, [true, false, true]);
    assert_eq!(text(&a.children[0]), "\n  ");
}

// Sabotage: with read_attributes keeping only the last of two same-named attributes, this test fails.
#[test]
fn keeps_duplicate_attributes_and_prefixed_names_as_written() {
    let a = root("<p:a xmlns:p=\"urn:x\" id=\"a\" id=\"b\"/>");
    assert_eq!(a.name, "p:a");
    let pairs: Vec<(&str, &str)> = a
        .attributes
        .iter()
        .map(|attribute| (attribute.name.as_str(), attribute.value.as_str()))
        .collect();
    assert_eq!(pairs, [("xmlns:p", "urn:x"), ("id", "a"), ("id", "b")]);
}

// Sabotage: with Element's Drop leaving the children to the default recursive drop, dropping the tree overflows the stack and the test binary aborts.
#[test]
fn parses_and_drops_deep_nesting_without_touching_the_call_stack() {
    let depth = 100_000;
    let source = format!("{}{}", "<s>".repeat(depth), "</s>".repeat(depth));
    let tree = root(&source);
    let mut node = &tree;
    let mut seen = 1;
    while let Some(child) = node.children.first() {
        node = element(child);
        seen += 1;
    }
    assert_eq!(seen, depth);
    drop(tree);
    let unclosed = format!("{}<s/>{}", "<s>".repeat(depth), "</s>".repeat(depth - 1));
    assert_eq!(refusal(unclosed.as_bytes()).reason, Reason::UnclosedElement);
}

// ---- refusals around the root element

// Sabotage: with the missing-root refusal answering ContentBeforeRoot, this test fails.
#[test]
fn refuses_a_document_with_no_root_element() {
    assert_refused("", Reason::MissingRoot, 1, 1);
    assert_refused("  <!-- c -->  ", Reason::MissingRoot, 1, 15);
}

// Sabotage: with at_start_tag accepting any '<', this test fails.
#[test]
fn refuses_character_data_before_the_root_element() {
    assert_refused("text<a/>", Reason::ContentBeforeRoot, 1, 1);
    assert_refused("<1a/>", Reason::ContentBeforeRoot, 1, 1);
    assert_refused(
        "<!DOCTYPE a><!DOCTYPE a><a/>",
        Reason::ContentBeforeRoot,
        1,
        13,
    );
}

// Sabotage: with the content-after-root refusal never made, this test fails.
#[test]
fn refuses_content_after_the_root_element() {
    assert_refused("<a/>text", Reason::ContentAfterRoot, 1, 5);
    assert_refused("<a/><b/>", Reason::ContentAfterRoot, 1, 5);
    assert_refused("<a/><!DOCTYPE a>", Reason::ContentAfterRoot, 1, 5);
}

// Sabotage: with the invalid-encoding refusal placed at the start of the document, this test fails.
#[test]
fn refuses_bytes_that_are_not_utf8_where_they_start() {
    assert_refused_bytes(b"<a>\xED\xA0\x80</a>", Reason::InvalidEncoding, 1, 4);
    assert_refused_bytes(b"<a>\n\xFF</a>", Reason::InvalidEncoding, 2, 1);
}

// Sabotage: with parse refusing only a doubled byte order mark, this test fails.
#[test]
fn refuses_a_leading_byte_order_mark_as_the_reference_does() {
    assert_refused("\u{FEFF}<a/>", Reason::ByteOrderMark, 1, 1);
}

// ---- refusals of markup

// Sabotage: with skip_comment answering Ok at the end of the source, this test fails.
#[test]
fn refuses_an_unterminated_comment() {
    assert_refused("<a><!-- open", Reason::UnterminatedComment, 1, 13);
}

// Sabotage: with skip_comment searching for "-->" instead of "--", this test fails.
#[test]
fn refuses_a_double_hyphen_inside_a_comment() {
    assert_refused(
        "<a><!-- a -- b --></a>",
        Reason::DoubleHyphenInComment,
        1,
        11,
    );
}

// Sabotage: with the unterminated-instruction refusal answering UnterminatedComment, this test fails.
#[test]
fn refuses_an_unterminated_processing_instruction() {
    assert_refused(
        "<a><?pi x",
        Reason::UnterminatedProcessingInstruction,
        1,
        10,
    );
    assert_refused("<a><? x?></a>", Reason::ExpectedName, 1, 6);
}

// Sabotage: with the declaration read wherever it stands (the offset test dropped), this test fails.
#[test]
fn refuses_an_xml_declaration_anywhere_but_the_start() {
    assert_refused(
        "<a><?xml version='1.0'?></a>",
        Reason::MisplacedXmlDeclaration,
        1,
        4,
    );
    assert_refused(
        "  <?xml version='1.0'?><a/>",
        Reason::MisplacedXmlDeclaration,
        1,
        3,
    );
    let twice = "<?xml version='1.0'?><?xml version='1.0'?><a/>";
    assert_refused(twice, Reason::MisplacedXmlDeclaration, 1, 22);
}

// Sabotage: with the reserved-target refusal never made, this test fails.
#[test]
fn refuses_a_reserved_processing_instruction_target() {
    assert_refused("<?XML version='1.0'?><a/>", Reason::ReservedTarget, 1, 1);
    assert_eq!(root("<?xml-model href='x'?><a/>").name, "a");
}

// Sabotage: with every three-character target refused as reserved, this test fails.
#[test]
fn reads_the_processing_instructions_the_readme_names_as_differences() {
    assert_eq!(root("<?xmlfoo?><a/>").name, "a");
    assert_eq!(root("<?lab?><?amb x?><a><?pix?></a>").name, "a");
}

// Sabotage: with is_version accepting any value, this test fails.
#[test]
fn refuses_a_malformed_xml_declaration() {
    for source in [
        "<?xml?><a/>",
        "<?xml foo?><a/>",
        "<?xml encoding='UTF-8'?><a/>",
        "<?xml version='1.0' standalone='maybe'?><a/>",
        "<?xml version='2.0'?><a/>",
        "<?xml version='1.'?><a/>",
        "<?xml version='1.0' encoding='9x'?><a/>",
        "<?xml version='1.0' encoding=''?><a/>",
        "<?xml version='1.0\"?><a/>",
        "<?xml version='1.0'",
    ] {
        assert_refused(source, Reason::MalformedXmlDeclaration, 1, 1);
    }
    let full = "<?xml version=\"1.1\" encoding=\"utf-8\" standalone='yes' ?><a/>";
    assert_eq!(root(full).name, "a");
    assert_eq!(
        root("<?xml version = '1.0'\nstandalone=\"no\"?><a/>").name,
        "a"
    );
}

// Sabotage: with read_declaration accepting any encoding, this test fails.
#[test]
fn refuses_a_declared_encoding_other_than_utf8() {
    let source = "<?xml version='1.0' encoding='latin1'?><a/>";
    assert_refused(source, Reason::UnsupportedEncoding, 1, 1);
}

// Sabotage: with skip_doctype answering Ok when an end is missing, this test fails.
#[test]
fn refuses_an_unterminated_doctype() {
    assert_refused("<!DOCTYPE a [", Reason::UnterminatedDoctype, 1, 14);
    assert_refused("<!DOCTYPE a [<?pi x", Reason::UnterminatedDoctype, 1, 20);
    assert_refused(
        "<!DOCTYPE a [<!-- [ -->]><a/>",
        Reason::UnterminatedDoctype,
        1,
        30,
    );
}

// Sabotage: with the ambiguous-doctype refusal never made, this test fails.
#[test]
fn refuses_a_doctype_whose_two_ends_hold_markup_between_them() {
    let source = "<!DOCTYPE a [<!-- ] --><b>]><a/>";
    assert_refused(source, Reason::AmbiguousDoctype, 1, 1);
}

// Sabotage: with skip_doctype moving to the bracket end, this test fails.
#[test]
fn reads_what_follows_a_doctype_from_where_the_reference_ends_it() {
    let entity = "<!DOCTYPE a [ <!ENTITY x \"]>\"> ]><a/>";
    assert_refused(entity, Reason::ContentBeforeRoot, 1, 32);
    let system = "<!DOCTYPE a SYSTEM \"x>y\"><a/>";
    assert_refused(system, Reason::ContentBeforeRoot, 1, 23);
}

// ---- refusals inside tags

// Sabotage: with read_name answering ExpectedName at the end of the source, this test fails.
#[test]
fn refuses_a_document_that_ends_inside_a_tag() {
    assert_refused("<a", Reason::UnexpectedEnd, 1, 3);
    assert_refused("<a x=\"1", Reason::UnexpectedEnd, 1, 8);
    assert_refused("<a x", Reason::UnexpectedEnd, 1, 5);
    assert_refused("<a x=", Reason::UnexpectedEnd, 1, 6);
    assert_refused("<a x ", Reason::UnexpectedEnd, 1, 6);
    assert_refused("<a ", Reason::UnexpectedEnd, 1, 4);
    assert_refused("<a></", Reason::UnexpectedEnd, 1, 6);
    assert_refused("<a></a", Reason::UnexpectedEnd, 1, 7);
}

// Sabotage: with is_name_start admitting digits, this test fails.
#[test]
fn refuses_a_tag_with_no_name() {
    assert_refused("<a><1/></a>", Reason::ExpectedName, 1, 5);
    assert_refused("<a><!x/></a>", Reason::ExpectedName, 1, 5);
    assert_refused("<a\u{D7}/>", Reason::ExpectedWhitespace, 1, 3);
}

// Sabotage: with read_attribute accepting a missing '=', this test fails.
#[test]
fn refuses_an_attribute_with_no_equals_sign() {
    assert_refused("<a x/>", Reason::ExpectedEquals, 1, 5);
}

// Sabotage: with read_attribute taking '1' as a quote, this test fails.
#[test]
fn refuses_an_unquoted_attribute_value() {
    assert_refused("<a x=1/>", Reason::ExpectedQuote, 1, 6);
}

// Sabotage: with the expected-whitespace refusal never made, this test fails.
#[test]
fn refuses_attributes_with_no_whitespace_between_them() {
    assert_refused("<a x=\"1\"y=\"2\"/>", Reason::ExpectedWhitespace, 1, 9);
}

// Sabotage: with read_end_tag skipping to the next '>', this test fails.
#[test]
fn refuses_an_end_tag_with_more_than_a_name() {
    assert_refused("<a></a x>", Reason::ExpectedTagEnd, 1, 8);
}

// Sabotage: with the '<' refusal answering ExpectedQuote, this test fails.
#[test]
fn refuses_a_less_than_sign_in_an_attribute_value() {
    assert_refused("<a x=\"<\"/>", Reason::LtInAttributeValue, 1, 7);
}

// Sabotage: with read_end_tag closing the open element whatever the name, this test fails.
#[test]
fn refuses_a_mismatched_end_tag_at_its_lt() {
    assert_refused("<a>\n  <b></c>\n</a>", Reason::MismatchedEndTag, 2, 6);
    let error = refusal(b"<a><b></c></a>");
    assert!(
        error.message.starts_with("end tag </c> where </b> was due"),
        "{}",
        error.message
    );
}

// Sabotage: with step naming the outermost open element, this test fails.
#[test]
fn refuses_an_element_left_open_naming_it() {
    assert_refused("<a><b>", Reason::UnclosedElement, 1, 7);
    let error = refusal(b"<a><b>");
    assert!(
        error.message.starts_with("element <b> is never closed"),
        "{}",
        error.message
    );
}

// ---- refusals of references and characters

// Sabotage: with read_reference refusing every name outside the five, this test fails.
#[test]
fn keeps_an_undeclared_entity_reference_as_its_literal_text_with_no_doctype() {
    assert_eq!(text_value("<a>x &nbsp; y</a>"), "x &nbsp; y");
    assert_eq!(attribute_value("<a b=\"&copy;\"/>"), "&copy;");
}

// Sabotage: with the is_whole_name check dropped from read_reference, this test fails.
#[test]
fn refuses_a_reference_whose_body_is_not_a_name() {
    assert_refused("<a>&1x;</a>", Reason::MalformedReference, 1, 4);
    assert_refused("<!DOCTYPE a><a>&1x;</a>", Reason::MalformedReference, 1, 16);
    let error = refusal(b"<a>&1x;</a>");
    assert!(
        error.message.starts_with("&1x; does not name an entity"),
        "{}",
        error.message
    );
}

// Sabotage: with read_reference letting a body run to any ';', this test fails.
#[test]
fn refuses_an_ampersand_that_starts_no_reference() {
    assert_refused("<a>R&D</a>", Reason::MalformedReference, 1, 5);
    assert_refused("<a>&amp</a>", Reason::MalformedReference, 1, 4);
    assert_refused("<a>&;</a>", Reason::MalformedReference, 1, 4);
    assert_refused("<a>& b;</a>", Reason::MalformedReference, 1, 4);
    assert_refused("<a><b x=\"&c\"/>d;</a>", Reason::MalformedReference, 1, 10);
}

// Sabotage: with the is_xml_char check dropped from character_reference, this test fails.
#[test]
fn refuses_a_character_reference_to_no_xml_character() {
    for reference in [
        "&#0;",
        "&#x;",
        "&#;",
        "&#Xe9;",
        "&#12a;",
        "&#xG1;",
        "&#xFFFE;",
        "&#x110000;",
        "&#xD800;",
        "&#99999999999999999999;",
    ] {
        let source = format!("<a>{reference}</a>");
        assert_refused(&source, Reason::InvalidCharacterReference, 1, 4);
    }
}

// Sabotage: with is_excluded admitting U+0001 to U+0008, this test fails.
#[test]
fn refuses_a_character_xml_does_not_allow() {
    assert_refused("<a>\u{1}</a>", Reason::InvalidCharacter, 1, 4);
    assert_refused("<a x=\"\u{1}\"/>", Reason::InvalidCharacter, 1, 7);
    assert_refused("<a><![CDATA[\u{1}]]></a>", Reason::InvalidCharacter, 1, 13);
    assert_refused("<a>x\u{FFFF}</a>", Reason::InvalidCharacter, 1, 5);
    let error = refusal("<a>\u{B}</a>".as_bytes());
    assert!(
        error
            .message
            .starts_with("the character U+000B is not allowed"),
        "{}",
        error.message
    );
}

// Sabotage: with the "]]>" refusal dropped from read_character_data, this test fails.
#[test]
fn refuses_a_cdata_end_in_character_data() {
    assert_refused("<a>x]]>y</a>", Reason::CdataEndInText, 1, 5);
    assert_eq!(text_value("<a>x]]y</a>"), "x]]y");
}

// Sabotage: with read_cdata answering Ok at the end of the source, this test fails.
#[test]
fn refuses_an_unterminated_cdata_section() {
    assert_refused("<a><![CDATA[x", Reason::UnterminatedCdata, 1, 14);
    assert_refused("<a><![CDATA[x]", Reason::UnterminatedCdata, 1, 15);
}

// ---- references

// Sabotage: with "apos" dropped from predefined, this test fails.
#[test]
fn expands_the_five_predefined_entities_in_text_and_in_an_attribute_value() {
    for (reference, character) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
    ] {
        let a = root(&format!("<a x=\"{reference}\">{reference}</a>"));
        assert_eq!(a.attributes[0].value, character, "{reference}");
        assert_eq!(text(&a.children[0]), character, "{reference}");
    }
}

// Sabotage: with character_reference reading every body as decimal, this test fails.
#[test]
fn decodes_decimal_and_hex_character_references() {
    for (reference, character) in [
        ("&#65;", "A"),
        ("&#x41;", "A"),
        ("&#x6c;", "l"),
        ("&#x4A;", "J"),
        ("&#x1F4DA;", "\u{1F4DA}"),
        ("&#128218;", "\u{1F4DA}"),
        ("&#00065;", "A"),
    ] {
        assert_eq!(
            text_value(&format!("<a>{reference}</a>")),
            character,
            "{reference}"
        );
        let attribute = attribute_value(&format!("<a x=\"{reference}\"/>"));
        assert_eq!(attribute, character, "{reference}");
    }
}

// Sabotage: with read_reference refusing every name outside the five, this test fails.
#[test]
fn keeps_a_declared_entity_reference_as_its_literal_text() {
    assert_eq!(
        text_value("<!DOCTYPE a [<!ENTITY x \"y\">]><a>&x;</a>"),
        "&x;"
    );
    let attribute = attribute_value("<!DOCTYPE a [<!ENTITY x \"y\">]><a b=\"&x;\"/>");
    assert_eq!(attribute, "&x;");
}

// Sabotage: with read_reference dropping the ampersand of a kept reference, this test fails.
#[test]
fn keeps_a_reference_under_an_external_subset_as_its_literal_text() {
    let source = "<!DOCTYPE a SYSTEM \"x.dtd\"><a>t &foo; &amp; &#65;</a>";
    assert_eq!(text_value(source), "t &foo; & A");
}

// ---- normalization

// Sabotage: with read_value keeping a literal tab, this test fails.
#[test]
fn a_literal_tab_lf_or_cr_in_an_attribute_value_becomes_a_space() {
    assert_eq!(attribute_value("<a x=\"a\tb\"/>"), "a b");
    assert_eq!(attribute_value("<a x=\"a\nb\"/>"), "a b");
    assert_eq!(attribute_value("<a x=\"a\rb\"/>"), "a b");
}

// Sabotage: with line_break leaving the LF after a CR, this test fails.
#[test]
fn a_literal_cr_lf_pair_in_an_attribute_value_becomes_one_space() {
    assert_eq!(attribute_value("<a x=\"a\r\nb\"/>"), "a b");
    assert_eq!(attribute_value("<a x=\"a\r\n\r\nb\"/>"), "a  b");
}

// Sabotage: with read_value turning a decoded LF into a space, this test fails.
#[test]
fn a_character_reference_keeps_its_character_in_an_attribute_value() {
    assert_eq!(attribute_value("<a x=\"a&#10;b\"/>"), "a\nb");
    assert_eq!(attribute_value("<a x=\"a&#9;b\"/>"), "a\tb");
    assert_eq!(attribute_value("<a x=\"a&#xD;b\"/>"), "a\rb");
    assert_eq!(attribute_value("<a x=\"&#10;\n\"/>"), "\n ");
}

// Sabotage: with read_attribute trimming the value, this test fails.
#[test]
fn nothing_in_an_attribute_value_is_trimmed_or_collapsed() {
    assert_eq!(attribute_value("<a x=\"  a  \"/>"), "  a  ");
    assert_eq!(attribute_value("<a x=\"a   b\"/>"), "a   b");
    assert_eq!(attribute_value("<a x=\"a &lt; b\"/>"), "a < b");
}

// Sabotage: with read_value ending at either quote, this test fails.
#[test]
fn each_quote_style_holds_the_other() {
    let a = root("<a x='say \"hi\"' y=\"it's\"/>");
    let values: Vec<&str> = a
        .attributes
        .iter()
        .map(|attribute| attribute.value.as_str())
        .collect();
    assert_eq!(values, ["say \"hi\"", "it's"]);
}

// Sabotage: with read_character_data folding a CR to a CR, this test fails.
#[test]
fn a_literal_cr_lf_pair_or_lone_cr_in_text_folds_to_one_lf() {
    assert_eq!(text_value("<a>a\r\nb</a>"), "a\nb");
    assert_eq!(text_value("<a>c\rd</a>"), "c\nd");
    assert_eq!(text_value("<a>q\r\n\r\nr</a>"), "q\n\nr");
}

// Sabotage: with flush folding every CR in the finished run, this test fails.
#[test]
fn a_cr_decoded_from_a_reference_stays_a_cr_in_text() {
    assert_eq!(text_value("<a>e&#xD;f</a>"), "e\rf");
}

// Sabotage: with read_cdata taking a CR as plain text, this test fails.
#[test]
fn line_breaks_fold_inside_a_cdata_section() {
    assert_eq!(text_value("<a><![CDATA[g\r\nh]]></a>"), "g\nh");
}

// Sabotage: with text keeping a raw CR and flush folding the finished run, this test fails.
#[test]
fn a_cr_before_a_comment_or_ending_a_cdata_section_is_a_lone_cr() {
    assert_eq!(text_value("<a>i\r<!--c-->\nj</a>"), "i\n\nj");
    assert_eq!(text_value("<a><![CDATA[k\r]]>\nl</a>"), "k\n\nl");
}

// Sabotage: with read_character_data turning a tab into a space, this test fails.
#[test]
fn text_keeps_a_tab_and_an_lf() {
    assert_eq!(text_value("<a>m\tn</a>"), "m\tn");
    assert_eq!(text_value("<a>o\np</a>"), "o\np");
}

// ---- namespaces

// Sabotage: with resolve answering None for an unprefixed name, this test fails.
#[test]
fn resolves_the_default_namespace_and_inherits_it() {
    let scxml = root(&format!(
        "<scxml xmlns=\"{SCXML}\"><state><final/></state></scxml>"
    ));
    let state = element(&scxml.children[0]);
    let last = element(&state.children[0]);
    let found = [&scxml.namespace, &state.namespace, &last.namespace];
    assert!(
        found
            .iter()
            .all(|namespace| namespace.as_deref() == Some(SCXML)),
        "{found:?}"
    );
}

// Sabotage: with resolve answering the empty namespace when none is declared, this test fails.
#[test]
fn reads_a_document_with_no_declaration_in_no_namespace() {
    let scxml = root("<scxml version=\"1.0\"><state id=\"a\"/></scxml>");
    assert_eq!(scxml.namespace, None);
    assert_eq!(element(&scxml.children[0]).namespace, None);
}

// Sabotage: with open_tag inheriting the root's scope instead of the parent's, this test fails.
#[test]
fn scopes_a_declaration_to_its_elements_subtree() {
    let scxml = root(&format!(
        "<scxml xmlns=\"{SCXML}\"><a xmlns=\"urn:o\"><b/></a><c/></scxml>"
    ));
    let a = element(&scxml.children[0]);
    let found = [
        a.namespace.as_deref(),
        element(&a.children[0]).namespace.as_deref(),
        element(&scxml.children[1]).namespace.as_deref(),
    ];
    assert_eq!(found, [Some("urn:o"), Some("urn:o"), Some(SCXML)]);
}

// Sabotage: with declare reading an empty default declaration as undeclaring, this test fails.
#[test]
fn binds_the_empty_string_for_an_empty_declaration() {
    let scxml = root(&format!(
        "<scxml xmlns=\"{SCXML}\"><a xmlns=\"\"><b/></a><c/></scxml>"
    ));
    let a = element(&scxml.children[0]);
    let found = [
        a.namespace.as_deref(),
        element(&a.children[0]).namespace.as_deref(),
        element(&scxml.children[1]).namespace.as_deref(),
    ];
    assert_eq!(found, [Some(""), Some(""), Some(SCXML)]);
    let prefixed = root("<a xmlns:p=\"urn:p\"><p:b xmlns:p=\"\"/><p:c/></a>");
    let found: Vec<Option<&str>> = prefixed
        .children
        .iter()
        .map(|node| element(node).namespace.as_deref())
        .collect();
    assert_eq!(found, [Some(""), Some("urn:p")]);
}

// Sabotage: with resolve falling back to the default namespace for an undeclared prefix, this test fails.
#[test]
fn resolves_a_prefix_and_answers_none_for_an_undeclared_one() {
    let source = format!(
        "<s:scxml xmlns:s=\"{SCXML}\" xmlns=\"urn:d\" xmlns:=\"urn:e\"><s:state/><x:y/><plain/></s:scxml>"
    );
    let scxml = root(&source);
    assert_eq!(scxml.namespace.as_deref(), Some(SCXML));
    let found: Vec<Option<&str>> = scxml
        .children
        .iter()
        .map(|node| element(node).namespace.as_deref())
        .collect();
    assert_eq!(found, [Some(SCXML), None, Some("urn:d")]);
}

// Sabotage: with open_tag giving the children of a content element a fresh empty scope, this test fails.
#[test]
fn places_undeclared_markup_inside_content_in_the_enclosing_default_namespace() {
    let inner = "<scxml version=\"1.0\"><final/></scxml>";
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><invoke><content>{inner}</content></invoke></state></scxml>"
    );
    let scxml = root(&source);
    let state = element(&scxml.children[0]);
    let invoke = element(&state.children[0]);
    let content = element(&invoke.children[0]);
    let nested = element(&content.children[0]);
    assert_eq!(
        (nested.name.as_str(), nested.namespace.as_deref()),
        ("scxml", Some(SCXML))
    );
    assert_eq!(nested.location.slice(&source), Some(inner));
}

// ---- locations

const LOCATED: &str = "<a>\n  <s1 id=\"b\">\n    t\n  </s1>\n</a>";

// Sabotage: with mark starting a line at column 0, this test fails.
#[test]
fn puts_the_line_and_column_of_its_lt_on_every_element() {
    let a = root(LOCATED);
    let s1 = element(&a.children[1]);
    assert_eq!(span_of(a.location), [1, 1, 0, 5, 5, LOCATED.len()]);
    assert_eq!(span_of(s1.location), [2, 3, 6, 4, 8, 31]);
    assert_eq!(a.location.slice(LOCATED), Some(LOCATED));
    assert_eq!(
        s1.location.slice(LOCATED),
        Some("<s1 id=\"b\">\n    t\n  </s1>")
    );
}

// Sabotage: with read_attribute spanning from the value's start, this test fails.
#[test]
fn spans_an_attribute_with_its_quotes_and_its_value_inside_them() {
    let a = root(LOCATED);
    let id = &element(&a.children[1]).attributes[0];
    assert_eq!(span_of(id.location), [2, 7, 10, 2, 13, 16]);
    assert_eq!(span_of(id.value_location), [2, 11, 14, 2, 12, 15]);
    assert_eq!(id.value_location.slice(LOCATED), Some("b"));
}

// Sabotage: with take_run starting a run at the read position, this test fails.
#[test]
fn spans_a_text_run_from_the_end_of_one_tag_to_the_start_of_the_next() {
    let a = root(LOCATED);
    let run = location(&element(&a.children[1]).children[0]);
    assert_eq!(span_of(run), [2, 14, 17, 4, 3, 26]);
    let source = "<a><!-- c -->x<!-- d --></a>";
    let only = location(&root(source).children[0]);
    assert_eq!(only.slice(source), Some("<!-- c -->x<!-- d -->"));
}

// Sabotage: with mark counting every byte as a column, this test fails.
#[test]
fn counts_columns_in_code_points_and_offsets_in_bytes() {
    let a = root("<a>\u{1F4DA}\u{E9}<b/></a>");
    let b = span_of(location(&a.children[1]));
    assert_eq!(b[..3], [1, 6, 9]);
}

// Sabotage: with mark starting a line at a CR too, this test fails.
#[test]
fn starts_a_line_only_at_a_line_feed() {
    let a = root("<a>\r<b/></a>");
    let b = span_of(location(&a.children[1]));
    assert_eq!(b[..2], [1, 5]);
}

// Sabotage: with open_tag spanning a self-closing element from its start to its start, this test fails.
#[test]
fn spans_a_self_closing_element_as_its_tag() {
    let source = "<a> <b x='1' /> </a>";
    let b = location(&root(source).children[1]);
    assert_eq!(b.slice(source), Some("<b x='1' />"));
}

// Sabotage: with read_attribute requiring '=' right after the name, this test fails.
#[test]
fn tolerates_whitespace_around_the_equals_sign_and_before_the_end_of_an_end_tag() {
    let source = "<a id = \"x\" ></a >";
    let a = root(source);
    assert_eq!(a.attributes[0].value, "x");
    assert_eq!(a.attributes[0].location.slice(source), Some("id = \"x\""));
    assert_eq!(a.location.slice(source), Some(source));
}

// Sabotage: with mark keeping its count on a move backwards, this test fails.
#[test]
fn counts_the_position_again_after_a_move_backwards() {
    let mut cursor = Cursor::new("ab\ncd\u{E9}f");
    cursor.move_to_end();
    let end = cursor.mark();
    assert_eq!((end.offset, end.line, end.column), (8, 2, 5));
    cursor.move_to(1);
    let back = cursor.mark();
    assert_eq!((back.offset, back.line, back.column), (1, 1, 2));
    assert_refused(
        "<a>\n<!-- a -- b --></a>",
        Reason::DoubleHyphenInComment,
        2,
        8,
    );
}

// ---- the corpus and the no-panic promise

// Every SCXML document the vendored corpus carries: the `source` of every
// case of the three suites, then every `.scxml` file under `cases/`.
fn corpus_documents() -> Vec<(String, Vec<u8>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("conformance/statifier");
    let mut documents = Vec::new();
    for suite in ["scion", "w3c", "statifier"] {
        let path = root.join("corpus").join(format!("{suite}.json"));
        let bytes = std::fs::read(&path).expect("a corpus file");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("corpus JSON");
        for case in json["cases"].as_array().expect("a case list") {
            let id = case["id"].as_str().expect("a case id");
            let source = case["source"].as_str().expect("a case source");
            documents.push((format!("{suite}/{id}"), source.as_bytes().to_vec()));
        }
    }
    let mut pending = vec![root.join("cases")];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("a cases directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "scxml")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    for path in files {
        let name = path
            .strip_prefix(&root)
            .expect("under the corpus")
            .display()
            .to_string();
        documents.push((name, std::fs::read(&path).expect("a case file")));
    }
    documents
}

// Sabotage: with the XML declaration refused even at the start of the document, this test fails.
#[test]
fn reads_every_scxml_document_in_the_vendored_corpus() {
    let documents = corpus_documents();
    let refused: Vec<String> = documents
        .iter()
        .filter_map(|(name, source)| {
            parse(source)
                .err()
                .map(|error| format!("{name}: {}", error.message))
        })
        .collect();
    println!("corpus documents read: {}", documents.len());
    assert!(documents.len() > 300, "{}", documents.len());
    assert!(refused.is_empty(), "{refused:#?}");
}

// Documents crafted to reach every construct, so that cutting them short
// stops inside each.
const CRAFTED: &[&str] = &[
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\r\n<!DOCTYPE s [<!ENTITY e \"v\">]>\
     <!-- c --><?pi d?><p:s xmlns=\"urn:a\" xmlns:p=\"urn:b\" x=' &amp;&#x41;&#66;&e;\t\r\n'>\
     t&lt;\r<![CDATA[c\r\n]]]]><b/><!-- d --><?pi?>\u{E9}\u{1F4DA}</p:s >\n<!-- e -->",
    "<a>R&D</a>",
    "<a>&#xD800;</a>",
    "<a>&#99999999999;</a>",
    "<a x=\"<\"/>",
    "<!DOCTYPE a [<!-- ] --><b>]><a/>",
];

// Every byte prefix of every document parses to a value: a tree or a
// refusal inside the prefix, never a panic.
fn assert_every_prefix_answers(source: &[u8]) -> usize {
    for cut in 0..=source.len() {
        let prefix = &source[..cut];
        if let Err(error) = parse(prefix) {
            assert!(
                error.location.start_offset <= cut,
                "{prefix:?}: {}",
                error.message
            );
        }
    }
    source.len() + 1
}

// Sabotage: with step panicking at the end of the source inside an element, a cut document panics and this test fails.
#[test]
fn answers_every_byte_prefix_of_every_corpus_document_without_panicking() {
    let documents = corpus_documents();
    // The documents are shared out over the available cores; a panic in any
    // thread fails the join, and so the test.
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let share = documents.len().div_ceil(threads);
    let prefixes: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = documents
            .chunks(share)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|(_, source)| assert_every_prefix_answers(source))
                        .sum::<usize>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("every prefix answers"))
            .sum()
    });
    println!(
        "corpus prefixes read: {prefixes} over {} documents",
        documents.len()
    );
    assert!(documents.len() > 300, "{}", documents.len());
    let expected: usize = documents.iter().map(|(_, source)| source.len() + 1).sum();
    assert_eq!(prefixes, expected);
}

// Sabotage: with read_cdata panicking at the end of the source, a cut document panics and this test fails.
#[test]
fn answers_every_byte_prefix_of_every_crafted_document_without_panicking() {
    for source in CRAFTED {
        assert_every_prefix_answers(source.as_bytes());
    }
    assert!(read(CRAFTED[0]).is_ok());
}
