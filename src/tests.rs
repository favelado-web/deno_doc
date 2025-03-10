// Copyright 2020-2022 the Deno authors. All rights reserved. MIT license.

use crate::parser::DocParser;
use crate::printer::DocPrinter;
use deno_graph::source::MemoryLoader;
use deno_graph::source::Source;
use deno_graph::BuildOptions;
use deno_graph::CapturingModuleAnalyzer;
use deno_graph::DefaultModuleParser;
use deno_graph::GraphKind;
use deno_graph::ModuleGraph;
use deno_graph::ModuleSpecifier;
use pretty_assertions::assert_eq;
use serde_json::json;

type MaybeHeaders<S> = Option<Vec<(S, S)>>;

pub(crate) async fn setup<S: AsRef<str> + Copy>(
  root: S,
  sources: Vec<(S, MaybeHeaders<S>, S)>,
) -> (ModuleGraph, CapturingModuleAnalyzer, ModuleSpecifier) {
  let sources = sources
    .into_iter()
    .map(|(s, h, c)| {
      (
        s,
        Source::Module {
          specifier: s,
          maybe_headers: h,
          content: c,
        },
      )
    })
    .collect();
  let mut memory_loader = MemoryLoader::new(sources, vec![]);
  let root = ModuleSpecifier::parse(root.as_ref()).unwrap();
  let analyzer = create_analyzer();
  let mut graph = ModuleGraph::new(GraphKind::TypesOnly);
  graph
    .build(
      vec![root.clone()],
      &mut memory_loader,
      BuildOptions {
        module_analyzer: Some(&analyzer),
        ..Default::default()
      },
    )
    .await;
  (graph, analyzer, root)
}

macro_rules! assert_contains {
  ($string:expr, $($test:expr),+ $(,)?) => {
    let string = &$string; // This might be a function call or something
    if !($(string.contains($test))||+) {
      panic!("{:?} does not contain any of {:?}", string, [$($test),+]);
    }
  }
}

macro_rules! assert_not_contains {
  ($string:expr, $($test:expr),+ $(,)?) => {
    let string = &$string; // This might be a function call or something
    if !($(!string.contains($test))||+) {
      panic!("{:?} contained {:?}", string, [$($test),+]);
    }
  }
}

macro_rules! doc_test {
  ( $name:ident, $source:expr; $block:expr ) => {
    doc_test!($name, $source, false; $block);
  };

  ( $name:ident, $source:expr, private; $block:expr ) => {
    doc_test!($name, $source, true; $block);
  };

  ( $name:ident, $source:expr, $private:expr; $block:expr ) => {
    doc_test!($name, $source, $private; $block, vec![]);
  };

  ( $name:ident, $source:expr, $private:expr; $block:expr, $diagnostics:expr ) => {
    #[tokio::test]
    async fn $name() {
      use super::setup;

      let source_code = $source;
      let private = $private;

      let (graph, analyzer, specifier) = setup("file:///test.ts", vec![
        ("file:///test.ts", None, source_code)
      ]).await;
      let parser = DocParser::new(&graph, private, analyzer.as_capturing_parser()).unwrap();
      let entries = parser
        .parse(&specifier)
        .unwrap();

      #[allow(unused_variables)]
      let doc = DocPrinter::new(&entries, false, private).to_string();

      #[allow(clippy::redundant_closure_call)]
      ($block)(entries, doc);

      let actual_diagnostics = parser
        .diagnostics()
        .into_iter()
        .map(|d| format!("{}:{}:{} {:?}", d.location.filename, d.location.line, d.location.col, d.kind))
        .collect::<Vec<_>>();
      let expected_diagnostics: Vec<&str> = $diagnostics;
      assert_eq!(actual_diagnostics, expected_diagnostics, "Diagnostics match.");
    }
  };
}

macro_rules! contains_test {
  ( $name:ident, $source:expr;
    $( $contains:expr ),* $( ; $( $notcontains:expr ),* )? ) => {
    contains_test!($name, $source, false; $($contains),* $(;$($notcontains),*)?);
  };

  ( $name:ident, $source:expr, private;
    $( $contains:expr ),* $( ; $( $notcontains:expr ),* )? ) => {
    contains_test!($name, $source, true; $($contains),* $(;$($notcontains),*)?);
  };

  ( $name:ident, $source:expr, $private:expr;
    $( $contains:expr ),* $( ; $( $notcontains:expr ),* )? ) => {
    doc_test!($name, $source, $private; |_entries, doc: String| {
      $(
        assert_contains!(doc, $contains);
      )*
      $(
        $(
          assert_not_contains!(doc, $notcontains);
        )*
      )?
    });
  };
}

macro_rules! json_test {
  ( $name:ident, $source:expr; $json:tt ) => {
    json_test!($name, $source, false; $json);
  };

  ( $name:ident, $source:expr, private; $json:tt ) => {
    json_test!($name, $source, true; $json);
  };

  ( $name:ident, $source:expr, $private:expr; $json:tt ) => {
    json_test!($name, $source, $private; $json, vec![]);
  };

  ( $name:ident, $source:expr, $private:expr; $json:tt, $diagnostics:expr ) => {
    doc_test!($name, $source, $private; |entries, _doc| {
      let actual = serde_json::to_value(&entries).unwrap();
      let expected_json = json!($json);
      pretty_assertions::assert_eq!(actual, expected_json);
    }, $diagnostics);
  };
}

#[tokio::test]
async fn content_type_handling() {
  let sources = vec![(
    "https://example.com/a",
    Source::Module {
      specifier: "https://example.com/a",
      maybe_headers: Some(vec![(
        "content-type",
        "application/typescript; charset=utf-8",
      )]),
      content: r#"export interface A {
      a: string;
    }"#,
    },
  )];
  let mut memory_loader = MemoryLoader::new(sources, vec![]);
  let root = ModuleSpecifier::parse("https://example.com/a").unwrap();
  let analyzer = create_analyzer();
  let mut graph = ModuleGraph::new(GraphKind::TypesOnly);
  graph
    .build(
      vec![root.clone()],
      &mut memory_loader,
      BuildOptions {
        module_analyzer: Some(&analyzer),
        ..Default::default()
      },
    )
    .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&root)
    .unwrap();
  assert_eq!(entries.len(), 1);
}

#[tokio::test]
async fn types_header_handling() {
  let sources = vec![
    (
      "https://example.com/a.js",
      Source::Module {
        specifier: "https://example.com/a.js",
        maybe_headers: Some(vec![
          ("content-type", "application/javascript; charset=utf-8"),
          ("x-typescript-types", "./a.d.ts"),
        ]),
        content: r#"console.log("a");"#,
      },
    ),
    (
      "https://example.com/a.d.ts",
      Source::Module {
        specifier: "https://example.com/a.d.ts",
        maybe_headers: Some(vec![(
          "content-type",
          "application/typescript; charset=utf-8",
        )]),
        content: r#"export const a: "a";"#,
      },
    ),
  ];
  let mut memory_loader = MemoryLoader::new(sources, vec![]);
  let root = ModuleSpecifier::parse("https://example.com/a.js").unwrap();
  let analyzer = create_analyzer();
  let mut graph = ModuleGraph::new(GraphKind::TypesOnly);
  graph
    .build(
      vec![root.clone()],
      &mut memory_loader,
      BuildOptions {
        module_analyzer: Some(&analyzer),
        ..Default::default()
      },
    )
    .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&root)
    .unwrap();
  assert_eq!(
    serde_json::to_value(&entries).unwrap(),
    json!([{
      "kind": "variable",
      "name": "a",
      "location": {
        "filename": "https://example.com/a.d.ts",
        "line": 1,
        "col": 13
      },
      "declarationKind": "export",
      "variableDef": {
        "tsType": {
          "repr": "a",
          "kind": "literal",
          "literal": {
            "kind": "string",
            "string": "a"
          }
        },
        "kind": "const"
      }
    }])
  );
}

#[tokio::test]
async fn reexports() {
  let nested_reexport_source_code = r#"
/**
  * JSDoc for bar
  */
export const bar = "bar";

export default 42;
"#;
  let reexport_source_code = r#"
export { bar } from "./nested_reexport.ts";

/**
 * JSDoc for const
 */
export const foo = "foo";

export const fizz = "fizz";
"#;
  let test_source_code = r#"
export { default, foo as fooConst, bar as barReExport } from "./reexport.ts";
import { fizz as buzz } from "./reexport.ts";

/** JSDoc for function */
export function fooFn(a: number) {
  return a;
}
"#;
  let (graph, analyzer, specifier) = setup(
    "file:///test.ts",
    vec![
      ("file:///test.ts", None, test_source_code),
      ("file:///reexport.ts", None, reexport_source_code),
      (
        "file:///nested_reexport.ts",
        None,
        nested_reexport_source_code,
      ),
    ],
  )
  .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();

  let expected_json = json!([
    {
      "kind": "variable",
      "name": "fooConst",
      "location": {
        "filename": "file:///reexport.ts",
        "line": 7,
        "col": 13
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "JSDoc for const",
      },
      "variableDef": {
        "tsType": {
          "repr": "foo",
          "kind": "literal",
          "literal": {
            "kind": "string",
            "string": "foo"
          }
        },
        "kind": "const"
      }
    },
    {
      "kind": "variable",
      "name": "barReExport",
      "location": {
        "filename": "file:///nested_reexport.ts",
        "line": 5,
        "col": 13
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "JSDoc for bar",
      },
      "variableDef": {
        "tsType": {
          "repr": "bar",
          "kind": "literal",
          "literal": {
            "kind": "string",
            "string": "bar"
          }
        },
        "kind": "const"
      }
    },
    {
      "kind": "function",
      "name": "fooFn",
      "location": {
        "filename": "file:///test.ts",
        "line": 6,
        "col": 0
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "JSDoc for function",
      },
      "functionDef": {
        "params": [
            {
              "name": "a",
              "kind": "identifier",
              "optional": false,
              "tsType": {
                "keyword": "number",
                "kind": "keyword",
                "repr": "number",
              },
            }
        ],
        "typeParams": [],
        "returnType": null,
        "hasBody": true,
        "isAsync": false,
        "isGenerator": false
      },
    },
    {
      "kind": "import",
      "name": "buzz",
      "location": {
        "filename": "file:///test.ts",
        "line": 3,
        "col": 0
      },
      "declarationKind": "private",
      "importDef": {
        "src": "file:///reexport.ts",
        "imported": "fizz",
      }
    }
  ]);
  let actual = serde_json::to_value(&entries).unwrap();
  assert_eq!(actual, expected_json);

  assert!(DocPrinter::new(&entries, false, false)
    .to_string()
    .as_str()
    .contains("function fooFn(a: number)"));
}

#[tokio::test]
async fn reexports_has_same_name() {
  let reexport_source_code = r#"
export interface Hello {}
export class Hello {}
"#;
  let test_source_code = r#"
export { Hello } from "./reexport.ts";
"#;

  let (graph, analyzer, specifier) = setup(
    "file:///test.ts",
    vec![
      ("file:///test.ts", None, test_source_code),
      ("file:///reexport.ts", None, reexport_source_code),
    ],
  )
  .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();

  let expected_json = json!([
    {
      "kind": "interface",
      "name": "Hello",
      "location": {
        "filename": "file:///reexport.ts",
        "line": 2,
        "col": 0
      },
      "declarationKind": "export",
      "interfaceDef": {
        "extends": [],
        "methods": [],
        "properties": [],
        "callSignatures": [],
        "indexSignatures": [],
        "typeParams": []
      }
    },
    {
      "kind": "class",
      "name": "Hello",
      "location": {
        "filename": "file:///reexport.ts",
        "line": 3,
        "col": 0
      },
      "declarationKind": "export",
      "classDef": {
        "isAbstract": false,
        "constructors": [],
        "properties": [],
        "indexSignatures": [],
        "methods": [],
        "extends": null,
        "implements": [],
        "typeParams": [],
        "superTypeParams": []
      }
    }
  ]);
  let actual = serde_json::to_value(&entries).unwrap();
  assert_eq!(actual, expected_json);

  let output = DocPrinter::new(&entries, false, false).to_string();
  assert!(output.contains("class Hello"));
  assert!(output.contains("interface Hello"));
}

#[tokio::test]
async fn deep_reexports() {
  let foo_source_code = r#"export const foo: string = "foo";"#;
  let bar_source_code = r#"export * from "./foo.ts""#;
  let baz_source_code = r#"export * from "./bar.ts""#;

  let (graph, analyzer, specifier) = setup(
    "file:///baz.ts",
    vec![
      ("file:///foo.ts", None, foo_source_code),
      ("file:///bar.ts", None, bar_source_code),
      ("file:///baz.ts", None, baz_source_code),
    ],
  )
  .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();

  let expected_json = json!([
    {
      "kind": "variable",
      "name": "foo",
      "location": {
        "filename": "file:///foo.ts",
        "line": 1,
        "col": 13
      },
      "declarationKind": "export",
      "variableDef": {
        "tsType": {
          "repr": "string",
          "kind": "keyword",
          "keyword": "string"
        },
        "kind": "const"
      }
    }
  ]);
  let actual = serde_json::to_value(&entries).unwrap();
  assert_eq!(actual, expected_json);

  assert!(DocPrinter::new(&entries, false, false)
    .to_string()
    .contains("const foo"))
}

#[tokio::test]
async fn reexport_module_doc() {
  let mod_doc_source_code = r#"
/**
 * This is some module doc.
 *
 * @module
 */

/** a variable */
export const a = "a";
"#;
  let ns_source_code = r#"
export * as b from "./mod_doc.ts";
"#;
  let (graph, analyzer, specifier) = setup(
    "file:///ns.ts",
    vec![
      ("file:///ns.ts", None, ns_source_code),
      ("file:///mod_doc.ts", None, mod_doc_source_code),
    ],
  )
  .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();

  let actual = serde_json::to_value(&entries).unwrap();
  let expected = json!([
    {
      "kind": "namespace",
      "name": "b",
      "location": {
        "filename": "file:///ns.ts",
        "line": 2,
        "col": 7
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "This is some module doc.\n",
        "tags": [
          {
            "kind": "module"
          }
        ]
      },
      "namespaceDef": {
        "elements": [
          {
            "kind": "variable",
            "name": "a",
            "location": {
              "filename": "file:///mod_doc.ts",
              "line": 9,
              "col": 13
            },
            "declarationKind": "export",
            "jsDoc": {
              "doc": "a variable"
            },
            "variableDef": {
              "tsType": {
                "repr": "a",
                "kind": "literal",
                "literal": {
                  "kind": "string",
                  "string": "a"
                }
              },
              "kind": "const"
            }
          }
        ]
      }
    }
  ]);
  assert_eq!(actual, expected);
}

#[tokio::test]
async fn filter_nodes_by_name() {
  use crate::find_nodes_by_name_recursively;
  use crate::DocNodeKind;
  let source_code = r#"
export namespace Deno {
  export class Buffer {}
  export function test(options: object): void;
  export function test(name: string, fn: Function): void;
  export function test(name: string | object, fn?: Function): void {}
}

export namespace Deno {
  export namespace Inner {
    export function a(): void {}
    export const b = 100;
  }

  export interface Conn {
    rid: number;
    closeWrite(): void;
  }

  export class Process {
    readonly pid: number;
    output(): Promise<Uint8Array>;
  }
}
"#;
  let (graph, analyzer, specifier) = setup(
    "file:///test.ts",
    vec![("file:///test.ts", None, source_code)],
  )
  .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse(&specifier)
    .unwrap();

  // Namespace
  let found =
    find_nodes_by_name_recursively(entries.clone(), "Deno".to_string());
  assert_eq!(found.len(), 1);
  assert_eq!(found[0].name, "Deno".to_string());

  // Overloaded functions
  let found =
    find_nodes_by_name_recursively(entries.clone(), "Deno.test".to_string());
  assert_eq!(found.len(), 3);
  assert_eq!(found[0].name, "test".to_string());
  assert_eq!(found[1].name, "test".to_string());
  assert_eq!(found[2].name, "test".to_string());

  // Nested namespace
  let found =
    find_nodes_by_name_recursively(entries.clone(), "Deno.Inner.a".to_string());
  assert_eq!(found.len(), 1);
  assert_eq!(found[0].name, "a".to_string());

  // Interface property
  let found = find_nodes_by_name_recursively(
    entries.clone(),
    "Deno.Conn.rid".to_string(),
  );
  assert_eq!(found.len(), 1);
  assert_eq!(found[0].name, "rid".to_string());
  assert_eq!(found[0].kind, DocNodeKind::Variable);

  // Interface method
  let found = find_nodes_by_name_recursively(
    entries.clone(),
    "Deno.Conn.closeWrite".to_string(),
  );
  assert_eq!(found.len(), 1);
  assert_eq!(found[0].name, "closeWrite".to_string());
  assert_eq!(found[0].kind, DocNodeKind::Function);

  // Class property
  let found = find_nodes_by_name_recursively(
    entries.clone(),
    "Deno.Process.pid".to_string(),
  );
  assert_eq!(found.len(), 1);
  assert_eq!(found[0].name, "pid".to_string());
  assert_eq!(found[0].kind, DocNodeKind::Variable);

  // Class method
  let found = find_nodes_by_name_recursively(
    entries.clone(),
    "Deno.Process.output".to_string(),
  );
  assert_eq!(found.len(), 1);
  assert_eq!(found[0].name, "output".to_string());
  assert_eq!(found[0].kind, DocNodeKind::Function);

  // No match
  let found =
    find_nodes_by_name_recursively(entries.clone(), "Deno.test.a".to_string());
  assert_eq!(found.len(), 0);

  let found = find_nodes_by_name_recursively(entries, "a.b.c".to_string());
  assert_eq!(found.len(), 0);
}

#[tokio::test]
async fn exports_imported_earlier() {
  let foo_source_code = r#"export const foo: string = "foo";"#;
  let test_source_code = r#"
  import { foo } from "./foo.ts";

  export { foo };
  "#;

  let (graph, analyzer, specifier) = setup(
    "file:///test.ts",
    vec![
      ("file:///foo.ts", None, foo_source_code),
      ("file:///test.ts", None, test_source_code),
    ],
  )
  .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();

  let expected_json = json!([
    {
      "kind": "variable",
      "name": "foo",
      "location": {
        "filename": "file:///foo.ts",
        "line": 1,
        "col": 13
      },
      "declarationKind": "export",
      "variableDef": {
        "tsType": {
          "repr": "string",
          "kind": "keyword",
          "keyword": "string"
        },
        "kind": "const"
      }
    },
    {
      "kind": "import",
      "name": "foo",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 2,
      },
      "declarationKind": "private",
      "importDef": {
        "src": "file:///foo.ts",
        "imported": "foo",
      },
    },
  ]);
  let actual = serde_json::to_value(&entries).unwrap();
  assert_eq!(actual, expected_json);
}

#[tokio::test]
async fn exports_imported_earlier_renamed() {
  let foo_source_code = r#"export const foo: string = "foo";"#;
  let test_source_code = r#"
  import { foo as f } from "./foo.ts";

  export { f };
  "#;

  let (graph, analyzer, specifier) = setup(
    "file:///test.ts",
    vec![
      ("file:///foo.ts", None, foo_source_code),
      ("file:///test.ts", None, test_source_code),
    ],
  )
  .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();

  let expected_json = json!([
    {
      "kind": "variable",
      "name": "f",
      "location": {
        "filename": "file:///foo.ts",
        "line": 1,
        "col": 13
      },
      "declarationKind": "export",
      "variableDef": {
        "tsType": {
          "repr": "string",
          "kind": "keyword",
          "keyword": "string"
        },
        "kind": "const"
      }
    },
    {
      "kind": "import",
      "name": "f",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 2
      },
      "declarationKind": "private",
      "importDef": {
        "src": "file:///foo.ts",
        "imported": "foo"
      }
    }
  ]);
  let actual = serde_json::to_value(&entries).unwrap();
  assert_eq!(actual, expected_json);
}

#[tokio::test]
async fn exports_imported_earlier_default() {
  let foo_source_code = r#"const foo: string = "foo";
  export default foo;"#;
  let test_source_code = r#"
  import foo from "./foo.ts";

  export { foo };
  "#;

  let (graph, analyzer, specifier) = setup(
    "file:///test.ts",
    vec![
      ("file:///foo.ts", None, foo_source_code),
      ("file:///test.ts", None, test_source_code),
    ],
  )
  .await;
  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();

  let expected_json = json!([
    {
      "kind": "variable",
      "name": "foo",
      "location": {
        "filename": "file:///foo.ts",
        "line": 1,
        "col": 6
      },
      "declarationKind": "export",
      "variableDef": {
        "tsType": {
          "repr": "string",
          "kind": "keyword",
          "keyword": "string"
        },
        "kind": "const"
      }
    },
    {
      "kind": "import",
      "name": "foo",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 2
      },
      "declarationKind": "private",
      "importDef": {
        "src": "file:///foo.ts",
        "imported": "default"
      }
    }
  ]);
  let actual = serde_json::to_value(&entries).unwrap();
  assert_eq!(actual, expected_json);
}

#[tokio::test]
async fn exports_imported_earlier_private() {
  let foo_source_code = r#"export const foo: string = "foo";"#;
  let test_source_code = r#"
  import { foo } from "./foo.ts";

  export { foo };
  "#;

  let (graph, analyzer, specifier) = setup(
    "file:///test.ts",
    vec![
      ("file:///foo.ts", None, foo_source_code),
      ("file:///test.ts", None, test_source_code),
    ],
  )
  .await;
  let entries = DocParser::new(&graph, true, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();

  let expected_json = json!([
    {
      "kind": "variable",
      "name": "foo",
      "location": {
        "filename": "file:///foo.ts",
        "line": 1,
        "col": 13
      },
      "declarationKind": "export",
      "variableDef": {
        "tsType": {
          "repr": "string",
          "kind": "keyword",
          "keyword": "string"
        },
        "kind": "const"
      }
    },
    {
      "kind": "import",
      "name": "foo",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 2,
      },
      "declarationKind": "private",
      "importDef": {
        "src": "file:///foo.ts",
        "imported": "foo",
      },
    },
  ]);
  let actual = serde_json::to_value(&entries).unwrap();
  assert_eq!(actual, expected_json);
}

#[tokio::test]
async fn variable_syntax() {
  let (graph, analyzer, specifier) = setup(
    "file:///foo.ts",
    vec![
      ("file:///foo.ts", None, "export * from './bar.tsx'"),
      ("file:///bar.tsx", None, "export default <foo>bar</foo>"),
    ],
  )
  .await;

  // This just needs to not throw a syntax error
  DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();
}

#[tokio::test]
async fn json_module() {
  let (graph, analyzer, specifier) = setup(
    "file:///foo.ts",
    vec![
      ("file:///foo.ts", None, "export { default as configFile } from './bar.json' assert { type: 'json' };"),
      ("file:///bar.json", None, r#"{ "a": 5, "b": "text", "c": null, "d": [1, 2], "e": { "a": 1 } }"#),
    ],
  )
  .await;

  let entries = DocParser::new(&graph, false, analyzer.as_capturing_parser())
    .unwrap()
    .parse_with_reexports(&specifier)
    .unwrap();

  let expected_json = json!([
    {
      "kind": "variable",
      "name": "configFile",
      "location": {
        "filename": "file:///bar.json",
        "line": 1,
        "col": 0
      },
      "declarationKind": "export",
      "variableDef": {
        "tsType": {
          "repr": "",
          "kind": "typeLiteral",
          "typeLiteral": {
            "methods": [],
            "properties": [{
              "name": "a",
              "params": [],
              "computed": false,
              "optional": false,
              "tsType": {
                "repr": "5",
                "kind": "literal",
                "literal": {
                  "kind": "number",
                  "number": 5.0,
                },
              },
              "typeParams": []
            }, {
              "name": "b",
              "params": [],
              "computed": false,
              "optional": false,
              "tsType": {
                "repr": "text",
                "kind": "literal",
                "literal": {
                  "kind": "string",
                  "string": "text",
                },
              },
              "typeParams": []
            }, {
              "name": "c",
              "params": [],
              "computed": false,
              "optional": false,
              "tsType": {
                "repr": "null",
                "kind": "keyword",
                "keyword": "null",
              },
              "typeParams": []
            }, {
              "name": "d",
              "params": [],
              "computed": false,
              "optional": false,
              "tsType": {
                "repr": "unknown[]",
                "kind": "array",
                "array": {
                  "repr": "unknown",
                  "kind": "keyword",
                  "keyword": "unknown",
                },
              },
              "typeParams": []
            }, {
              "name": "e",
              "params": [],
              "computed": false,
              "optional": false,
              "tsType": {
                "repr": "",
                "kind": "typeLiteral",
                "typeLiteral": {
                  "methods": [],
                  "properties": [{
                    "name": "a",
                    "params": [],
                    "computed": false,
                    "optional": false,
                    "tsType": {
                      "repr": "1",
                      "kind": "literal",
                      "literal": {
                        "kind": "number",
                        "number": 1.0,
                      },
                    },
                    "typeParams": []
                  }],
                  "callSignatures": [],
                  "indexSignatures": [],
                },
              },
              "typeParams": []
            }],
            "callSignatures": [],
            "indexSignatures": [],
          },
        },
        "kind": "var",
      },
    },
  ]);
  let actual = serde_json::to_value(&entries).unwrap();
  assert_eq!(actual, expected_json);
}

mod serialization {
  use crate::*;

  json_test!(module_docs,
  r#"/* a non-jsdoc comment */

/**
 * A leading JSDoc comment
 * @module
 */

/** One associated with a class */
export class A {}
    "#;
  [
    {
      "kind": "moduleDoc",
      "name": "",
      "location": {
        "filename": "file:///test.ts",
        "line": 3,
        "col": 0,
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "A leading JSDoc comment",
        "tags": [{
          "kind": "module"
        }],
      }
    }, {
      "kind": "class",
      "name": "A",
      "location": {
        "filename": "file:///test.ts",
        "line": 9,
        "col": 0,
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "One associated with a class",
      },
      "classDef": {
        "isAbstract": false,
        "constructors": [],
        "properties": [],
        "indexSignatures": [],
        "methods": [],
        "extends": null,
        "implements": [],
        "typeParams": [],
        "superTypeParams": []
      }
    }
  ]);

  json_test!(module_docs_ignore,
  r#"/* a non-jsdoc comment */

/**
 * A leading JSDoc comment
 * @module
 * @ignore
 */

/** One associated with a class */
export class A {}
    "#;
  []);

  json_test!(declare_namespace,
    r#"
/** Namespace JSdoc */
declare namespace RootNs {
    declare const a = "a";

    /** Nested namespace JSDoc */
    declare namespace NestedNs {
      declare enum Foo {
        a = 1,
        b = 2,
        c = 3,
      }
    }
}
    "#;
    [{
    "kind": "namespace",
    "name": "RootNs",
    "location": {
      "filename": "file:///test.ts",
      "line": 3,
      "col": 0
    },
    "declarationKind": "declare",
    "jsDoc": {
      "doc": "Namespace JSdoc",
    },
    "namespaceDef": {
      "elements": [
        {
          "kind": "variable",
          "name": "a",
          "location": {
            "filename": "file:///test.ts",
            "line": 4,
            "col": 18
          },
          "declarationKind": "declare",
          "variableDef": {
            "tsType": {
              "repr": "a",
              "kind": "literal",
              "literal": {
                "kind": "string",
                "string": "a"
              }
            },
            "kind": "const"
          }
        },
        {
          "kind": "namespace",
          "name": "NestedNs",
          "location": {
            "filename": "file:///test.ts",
            "line": 7,
            "col": 4
          },
          "jsDoc": {
            "doc": "Nested namespace JSDoc",
          },
          "declarationKind": "declare",
          "namespaceDef": {
            "elements": [
              {
                "kind": "enum",
                "name": "Foo",
                "location": {
                  "filename": "file:///test.ts",
                  "line": 8,
                  "col": 6
                },
                "declarationKind": "declare",
                "enumDef": {
                  "members": [
                    {
                      "name": "a",
                      "init": {
                        "repr": "1",
                        "kind": "literal",
                        "literal": {
                          "kind": "number",
                          "number": 1.0,
                        }
                      },
                      "location": {
                        "filename": "file:///test.ts",
                        "line": 9,
                        "col": 8,
                      },
                    },
                    {
                      "name": "b",
                      "init": {
                        "repr": "2",
                        "kind": "literal",
                        "literal": {
                          "kind": "number",
                          "number": 2.0,
                        }
                      },
                      "location": {
                        "filename": "file:///test.ts",
                        "line": 10,
                        "col": 8,
                      },
                    },
                    {
                      "name": "c",
                      "init": {
                        "repr": "3",
                        "kind": "literal",
                        "literal": {
                          "kind": "number",
                          "number": 3.0,
                        }
                      },
                      "location": {
                        "filename": "file:///test.ts",
                        "line": 11,
                        "col": 8,
                      },
                    }
                  ]
                }
              }
            ]
          }
        }
      ]
    }
  }]);

  json_test!(declare_namespace_ignore,
    r#"
/** Namespace JSdoc */
declare namespace RootNs {
    declare const a = "a";

    /** Nested namespace JSDoc
     * @ignore
     */
    declare namespace NestedNs {
      declare enum Foo {
        a = 1,
        b = 2,
        c = 3,
      }
    }
}
    "#;
    [{
    "kind": "namespace",
    "name": "RootNs",
    "location": {
      "filename": "file:///test.ts",
      "line": 3,
      "col": 0
    },
    "declarationKind": "declare",
    "jsDoc": {
      "doc": "Namespace JSdoc",
    },
    "namespaceDef": {
      "elements": [
        {
          "kind": "variable",
          "name": "a",
          "location": {
            "filename": "file:///test.ts",
            "line": 4,
            "col": 18
          },
          "declarationKind": "declare",
          "variableDef": {
            "tsType": {
              "repr": "a",
              "kind": "literal",
              "literal": {
                "kind": "string",
                "string": "a"
              }
            },
            "kind": "const"
          }
        }
      ]
    }
  }]);

  json_test!(structured_jsdoc,
  r#"
/** Class doc */
export class A {
  /** @private */
  p = false;

  /**
   * Some leading documentation here.
   *
   * @param {string} name some comment
   * @param {string} a    some other comment that
   *                      spans two lines
   * @param {number} b    a number
   */
  constructor(name, a, b) {}

  /**
   * @returns {Promise<void>}
   */
  a() {}
}
  "#;
  [{
    "kind": "class",
    "name": "A",
    "location": {
      "filename": "file:///test.ts",
      "line": 3,
      "col": 0,
    },
    "jsDoc": {
      "doc": "Class doc",
    },
    "declarationKind": "export",
    "classDef": {
      "isAbstract": false,
      "constructors": [
        {
          "jsDoc": {
            "doc": "Some leading documentation here.\n",
            "tags": [
              {
                "kind": "param",
                "name": "name",
                "type": "string",
                "doc": "some comment",
              },
              {
                "kind": "param",
                "name": "a",
                "type": "string",
                "doc": "some other comment that\nspans two lines",
              },
              {
                "kind": "param",
                "name": "b",
                "type": "number",
                "doc": "a number",
              },
            ],
          },
          "accessibility": null,
          "hasBody": true,
          "name": "constructor",
          "params": [
            {
              "kind": "identifier",
              "name": "name",
              "optional": false,
              "tsType": null,
            },
            {
              "kind": "identifier",
              "name": "a",
              "optional": false,
              "tsType": null,
            },
            {
              "kind": "identifier",
              "name": "b",
              "optional": false,
              "tsType": null,
            }
          ],
          "location": {
            "filename": "file:///test.ts",
            "line": 15,
            "col": 2,
          }
        }
      ],
      "properties": [
        {
          "jsDoc": {
            "tags": [
              {
                "kind": "private"
              }
            ]
          },
          "tsType": {
            "repr": "boolean",
            "kind": "keyword",
            "keyword": "boolean",
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "p",
          "location": {
            "filename": "file:///test.ts",
            "line": 5,
            "col": 2,
          }
        }
      ],
      "indexSignatures": [],
      "methods": [
        {
          "jsDoc": {
            "tags": [
              {
                "kind": "return",
                "type": "Promise<void>"
              }
            ]
          },
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "a",
          "kind": "method",
          "functionDef": {
            "params": [],
            "returnType": null,
            "hasBody": true,
            "isAsync": false,
            "isGenerator": false,
            "typeParams": [],
          },
          "location": {
            "filename": "file:///test.ts",
            "line": 20,
            "col": 2,
          }
        }
      ],
      "extends": null,
      "implements": [],
      "typeParams": [],
      "superTypeParams": [],
    }
  }]);

  json_test!(export_class,
   r#"
/** Class doc */
export class Foobar extends Fizz implements Buzz, Aldrin {
    private private1?: boolean;
    protected protected1: number;
    public public1: boolean;
    public2: number;
    override public3: string;
    m = new Map<string, string>();
    s = "foo";
    a = [1, "foo"];
    fn = (a: string): void => {};

    /** Constructor js doc */
    constructor(name: string, private private2: number, protected protected2: number) {}

    /** Async foo method */
    async foo(): Promise<void> {
        //
    }

    /** Sync bar method */
    bar?(): void {
        //
    }

    override s() {}
}
  "#;
  [{
    "kind": "class",
    "name": "Foobar",
    "location": {
      "filename": "file:///test.ts",
      "line": 3,
      "col": 0
    },
    "declarationKind": "export",
    "jsDoc": {
      "doc": "Class doc",
    },
    "classDef": {
      "isAbstract": false,
      "extends": "Fizz",
      "implements": [
        {
          "repr": "Buzz",
          "kind": "typeRef",
          "typeRef": {
            "typeParams": null,
            "typeName": "Buzz"
          }
        },
        {
          "repr": "Aldrin",
          "kind": "typeRef",
          "typeRef": {
            "typeParams": null,
            "typeName": "Aldrin"
          }
        }
      ],
      "typeParams": [],
      "superTypeParams": [],
      "constructors": [
        {
          "jsDoc": {
            "doc": "Constructor js doc",
          },
          "accessibility": null,
          "hasBody": true,
          "name": "constructor",
          "params": [
            {
              "name": "name",
              "kind": "identifier",
              "optional": false,
              "tsType": {
                "repr": "string",
                "kind": "keyword",
                "keyword": "string"
              }
            },
            {
              "accessibility": "private",
              "name": "private2",
              "kind": "identifier",
              "optional": false,
              "tsType": {
                "repr": "number",
                "kind": "keyword",
                "keyword": "number"
              }
            },
            {
              "accessibility": "protected",
              "name": "protected2",
              "kind": "identifier",
              "optional": false,
              "tsType": {
                "repr": "number",
                "kind": "keyword",
                "keyword": "number"
              }
            }
          ],
          "location": {
            "filename": "file:///test.ts",
            "line": 15,
            "col": 4
          }
        }
      ],
      "properties": [
        {
          "tsType": {
              "repr": "boolean",
              "kind": "keyword",
              "keyword": "boolean"
          },
          "readonly": false,
          "accessibility": "private",
          "optional": true,
          "isAbstract": false,
          "isStatic": false,
          "name": "private1",
          "location": {
            "filename": "file:///test.ts",
            "line": 4,
            "col": 4
          }
        },
        {
          "tsType": {
            "repr": "number",
            "kind": "keyword",
            "keyword": "number"
          },
          "readonly": false,
          "accessibility": "protected",
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "protected1",
          "location": {
            "filename": "file:///test.ts",
            "line": 5,
            "col": 4
          }
        },
        {
          "tsType": {
            "repr": "boolean",
            "kind": "keyword",
            "keyword": "boolean"
          },
          "readonly": false,
          "accessibility": "public",
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "public1",
          "location": {
            "filename": "file:///test.ts",
            "line": 6,
            "col": 4
          }
        },
        {
          "tsType": {
            "repr": "number",
            "kind": "keyword",
            "keyword": "number"
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "public2",
          "location": {
            "filename": "file:///test.ts",
            "line": 7,
            "col": 4
          }
        },
        {
          "tsType": {
            "repr": "string",
            "kind": "keyword",
            "keyword": "string"
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "isOverride": true,
          "name": "public3",
          "location": {
            "filename": "file:///test.ts",
            "line": 8,
            "col": 4
          }
        },
        {
          "tsType": {
            "repr": "Map",
            "kind": "typeRef",
            "typeRef": {
              "typeParams": [
                {
                  "repr": "string",
                  "kind": "keyword",
                  "keyword": "string"
                },
                {
                  "repr": "string",
                  "kind": "keyword",
                  "keyword": "string"
                }
              ],
              "typeName": "Map",
            },
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "m",
          "location": {
            "filename": "file:///test.ts",
            "line": 9,
            "col": 4,
          }
        },
        {
          "tsType": {
            "repr": "string",
            "kind": "keyword",
            "keyword": "string",
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "s",
          "location": {
            "filename": "file:///test.ts",
            "line": 10,
            "col": 4,
          }
        },
        {
          "tsType": {
            "repr": "",
            "kind": "array",
            "array": {
              "repr": "",
              "kind": "union",
              "union": [
                {
                  "repr": "number",
                  "kind": "keyword",
                  "keyword": "number",
                }, {
                  "repr": "string",
                  "kind": "keyword",
                  "keyword": "string",
                }
              ]
            }
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "a",
          "location": {
            "filename": "file:///test.ts",
            "line": 11,
            "col": 4,
          }
        },
        {
          "tsType": {
            "repr": "",
            "kind": "fnOrConstructor",
            "fnOrConstructor": {
              "constructor": false,
              "tsType": {
                "repr": "void",
                "kind": "keyword",
                "keyword": "void"
              },
              "params": [
                {
                  "kind": "identifier",
                  "name": "a",
                  "optional": false,
                  "tsType": {
                    "repr": "string",
                    "kind": "keyword",
                    "keyword": "string",
                  }
                }
              ],
              "typeParams": []
            }
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "fn",
          "location": {
            "filename": "file:///test.ts",
            "line": 12,
            "col": 4,
          },
        }
      ],
      "indexSignatures": [],
      "methods": [
        {
          "jsDoc": {
            "doc": "Async foo method",
          },
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "foo",
          "kind": "method",
          "functionDef": {
            "params": [],
            "returnType": {
                "repr": "Promise",
                "kind": "typeRef",
                "typeRef": {
                  "typeParams": [
                    {
                      "repr": "void",
                      "kind": "keyword",
                      "keyword": "void"
                    }
                  ],
                  "typeName": "Promise"
                }
            },
            "typeParams": [],
            "hasBody": true,
            "isAsync": true,
            "isGenerator": false
          },
          "location": {
            "filename": "file:///test.ts",
            "line": 18,
            "col": 4
          }
        },
        {
          "jsDoc": {
            "doc": "Sync bar method",
          },
          "accessibility": null,
          "optional": true,
          "isAbstract": false,
          "isStatic": false,
          "name": "bar",
          "kind": "method",
          "functionDef": {
            "params": [],
            "returnType": {
              "repr": "void",
              "kind": "keyword",
              "keyword": "void"
            },
            "hasBody": true,
            "isAsync": false,
            "isGenerator": false,
            "typeParams": []
          },
          "location": {
            "filename": "file:///test.ts",
            "line": 23,
            "col": 4
          }
        },
        {
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "isOverride": true,
          "name": "s",
          "kind": "method",
          "functionDef": {
            "params": [],
            "returnType": null,
            "hasBody": true,
            "isAsync": false,
            "isGenerator": false,
            "typeParams": [],
          },
          "location": {
            "filename": "file:///test.ts",
            "line": 27,
            "col": 4,
          }
        }
      ]
    }
  }]);

  json_test!(
    export_class_object_extends,
    r#"
class Foo {}
const obj = { Foo }
    
export class Bar extends obj.Foo {}
  "#,
  false;
  [{
    "kind": "class",
    "name": "Bar",
    "location": {
      "filename": "file:///test.ts",
      "line": 5,
      "col": 0
    },
    "declarationKind": "export",
    "classDef": {
      "isAbstract": false,
      "constructors": [],
      "properties": [],
      "indexSignatures": [],
      "methods": [],
      "extends": "obj.Foo",
      "implements": [],
      "typeParams": [],
      "superTypeParams": []
    }
  }, {
    "kind": "variable",
    "name": "obj",
    "location": {
      "filename": "file:///test.ts",
      "line": 3,
      "col": 6,
    },
    "declarationKind": "private",
    "variableDef": {
      "tsType": {
        "repr": "",
        "kind": "typeLiteral",
        "typeLiteral": {
          "methods": [],
          "properties": [{
            "name": "Foo",
            "params": [],
            "computed": false,
            "optional": false,
            "tsType": null,
            "typeParams": [],
          }],
          "callSignatures": [],
          "indexSignatures": [],
        },
      },
      "kind": "const",
    },
  }], vec!["file:///test.ts:3:6 PrivateTypeRef"]
  );

  json_test!(export_class_ignore,
   r#"
/** Class doc */
export class Foobar extends Fizz implements Buzz, Aldrin {
    /** @ignore */
    private private1?: boolean;
    protected protected1: number;
    /** @ignore */
    public public1: boolean;
    public2: number;
    /** @ignore */
    override public3: string;
    /** @ignore */
    m = new Map<string, string>();
    s = "foo";
    a = [1, "foo"];
    fn = (a: string): void => {};

    /** Constructor js doc */
    /** @ignore */
    constructor(name: string, private private2: number, protected protected2: number) {}

    /** Async foo method */
    /** @ignore */
    async foo(): Promise<void> {
        //
    }

    /** Sync bar method */
    /** @ignore */
    bar?(): void {
        //
    }

    /** @ignore */
    override s() {}
}
  "#;
  [{
    "kind": "class",
    "name": "Foobar",
    "location": {
      "filename": "file:///test.ts",
      "line": 3,
      "col": 0
    },
    "declarationKind": "export",
    "jsDoc": {
      "doc": "Class doc",
    },
    "classDef": {
      "isAbstract": false,
      "extends": "Fizz",
      "constructors": [],
      "implements": [
        {
          "repr": "Buzz",
          "kind": "typeRef",
          "typeRef": {
            "typeParams": null,
            "typeName": "Buzz"
          }
        },
        {
          "repr": "Aldrin",
          "kind": "typeRef",
          "typeRef": {
            "typeParams": null,
            "typeName": "Aldrin"
          }
        }
      ],
      "typeParams": [],
      "superTypeParams": [],
      "properties": [
        {
          "tsType": {
            "repr": "number",
            "kind": "keyword",
            "keyword": "number"
          },
          "readonly": false,
          "accessibility": "protected",
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "protected1",
          "location": {
            "filename": "file:///test.ts",
            "line": 6,
            "col": 4
          }
        },
        {
          "tsType": {
            "repr": "number",
            "kind": "keyword",
            "keyword": "number"
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "public2",
          "location": {
            "filename": "file:///test.ts",
            "line": 9,
            "col": 4
          }
        },
        {
          "tsType": {
            "repr": "string",
            "kind": "keyword",
            "keyword": "string",
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "s",
          "location": {
            "filename": "file:///test.ts",
            "line": 14,
            "col": 4,
          }
        },
        {
          "tsType": {
            "repr": "",
            "kind": "array",
            "array": {
              "repr": "",
              "kind": "union",
              "union": [
                {
                  "repr": "number",
                  "kind": "keyword",
                  "keyword": "number",
                }, {
                  "repr": "string",
                  "kind": "keyword",
                  "keyword": "string",
                }
              ]
            }
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "a",
          "location": {
            "filename": "file:///test.ts",
            "line": 15,
            "col": 4,
          }
        },
        {
          "tsType": {
            "repr": "",
            "kind": "fnOrConstructor",
            "fnOrConstructor": {
              "constructor": false,
              "tsType": {
                "repr": "void",
                "kind": "keyword",
                "keyword": "void"
              },
              "params": [
                {
                  "kind": "identifier",
                  "name": "a",
                  "optional": false,
                  "tsType": {
                    "repr": "string",
                    "kind": "keyword",
                    "keyword": "string",
                  }
                }
              ],
              "typeParams": []
            }
          },
          "readonly": false,
          "accessibility": null,
          "optional": false,
          "isAbstract": false,
          "isStatic": false,
          "name": "fn",
          "location": {
            "filename": "file:///test.ts",
            "line": 16,
            "col": 4,
          },
        }
      ],
      "indexSignatures": [],
      "methods": []
    }
  }]);

  json_test!(export_class_decorators,
    r#"
@sealed
export class A {
  #x = "x";

  @format("Hello, %s")
  greeting: string;

  @configurable(false)
  get x() {
    return this.#x;
  }

  @enumerable(false)
  greet() {
    return "hello";
  }

  method(@readonly param: string) {}
}
"#;
    [{
      "kind": "class",
      "name": "A",
      "location": {
        "filename": "file:///test.ts",
        "line": 3,
        "col": 0,
      },
      "declarationKind": "export",
      "classDef": {
        "isAbstract": false,
        "constructors": [],
        "properties": [
          {
            "tsType": {
              "repr": "string",
              "kind": "keyword",
              "keyword": "string",
            },
            "readonly": false,
            "accessibility": null,
            "decorators": [
              {
                "name": "format",
                "args": [
                  "\"Hello, %s\"",
                ],
                "location": {
                  "filename": "file:///test.ts",
                  "line": 6,
                  "col": 3,
                }
              }
            ],
            "optional": false,
            "isAbstract": false,
            "isStatic": false,
            "name": "greeting",
            "location": {
              "filename": "file:///test.ts",
              "line": 6,
              "col": 2,
            }
          }
        ],
        "indexSignatures": [],
        "methods": [
          {
            "accessibility": null,
            "optional": false,
            "isAbstract": false,
            "isStatic": false,
            "name": "x",
            "kind": "getter",
            "functionDef": {
              "params": [],
              "returnType": null,
              "hasBody": true,
              "isAsync": false,
              "isGenerator": false,
              "typeParams": [],
              "decorators": [
                {
                  "name": "configurable",
                  "args": [
                    "false"
                  ],
                  "location": {
                    "filename": "file:///test.ts",
                    "line": 9,
                    "col": 3,
                  }
                }
              ]
            },
            "location": {
              "filename": "file:///test.ts",
              "line": 9,
              "col": 2,
            }
          }, {
            "accessibility": null,
            "optional": false,
            "isAbstract": false,
            "isStatic": false,
            "name": "greet",
            "kind": "method",
            "functionDef": {
              "params": [],
              "returnType": null,
              "hasBody": true,
              "isAsync": false,
              "isGenerator": false,
              "typeParams": [],
              "decorators": [
                {
                  "name": "enumerable",
                  "args": [
                    "false"
                  ],
                  "location": {
                    "filename": "file:///test.ts",
                    "line": 14,
                    "col": 3,
                  }
                }
              ]
            },
            "location": {
              "filename": "file:///test.ts",
              "line": 14,
              "col": 2,
            }
          }, {
            "accessibility": null,
            "optional": false,
            "isAbstract": false,
            "isStatic": false,
            "name": "method",
            "kind": "method",
            "functionDef": {
              "params": [
                {
                  "kind": "identifier",
                  "name": "param",
                  "optional": false,
                  "decorators": [
                    {
                      "name": "readonly",
                      "location": {
                        "filename": "file:///test.ts",
                        "line": 19,
                        "col": 10,
                      }
                    }
                  ],
                  "tsType": {
                    "repr": "string",
                    "kind": "keyword",
                    "keyword": "string",
                  }
                }
              ],
              "returnType": null,
              "hasBody": true,
              "isAsync": false,
              "isGenerator": false,
              "typeParams": [],
            },
            "location": {
              "filename": "file:///test.ts",
              "line": 19,
              "col": 2,
            }
          }
        ],
        "extends": null,
        "implements": [],
        "typeParams": [],
        "superTypeParams": [],
        "decorators": [
          {
            "name": "sealed",
            "location": {
              "filename": "file:///test.ts",
              "line": 2,
              "col": 1,
            }
          }
        ]
      }
    }]
  );

  json_test!(decorators_jsdoc,
  r#"
/** jsdoc */
@sealed
export class A {
  a: string = "a";
}
    "#;
  [
    {
      "kind": "class",
      "name": "A",
      "location": {
        "filename": "file:///test.ts",
        "line": 4,
        "col": 0,
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "jsdoc"
      },
      "classDef": {
        "isAbstract": false,
        "constructors": [],
        "properties": [
          {
            "tsType": {
              "repr": "string",
              "kind": "keyword",
              "keyword": "string",
            },
            "readonly": false,
            "accessibility": null,
            "optional": false,
            "isAbstract": false,
            "isStatic": false,
            "name": "a",
            "location": {
              "filename": "file:///test.ts",
              "line": 5,
              "col": 2,
            }
          }
        ],
        "indexSignatures": [],
        "methods": [],
        "extends": null,
        "implements": [],
        "typeParams": [],
        "superTypeParams": [],
        "decorators": [
          {
            "name": "sealed",
            "location": {
              "filename": "file:///test.ts",
              "line": 3,
              "col": 1,
            }
          }
        ]
      }
    }
  ]);

  json_test!(export_const_basic,
    r#"
/** Something about fizzBuzz */
export const fizzBuzz = "fizzBuzz";

export const env: {
  /** get doc */
  get(key: string): string | undefined;

  /** set doc */
  set(key: string, value: string): void;
}

export const num = 100;
export const bool = false;
export const bigint = 123n;
export const regex = /hello/;
export const date = new Date();
export const tpl1 = `foo`;
export const tpl2 = `Value: ${num}`;
/** @ignore */
export const tpl3 = `Value: ${num}`;
    "#;
  [
  {
    "kind":"variable",
    "name":"fizzBuzz",
    "location":{
      "filename":"file:///test.ts",
      "line":3,
      "col":13
    },
    "declarationKind": "export",
    "jsDoc": {
      "doc": "Something about fizzBuzz",
    },
    "variableDef":{
      "tsType": {
        "repr": "fizzBuzz",
        "kind": "literal",
        "literal": {
          "kind": "string",
          "string": "fizzBuzz"
        }
      },
      "kind":"const"
    }
  },
  {
    "kind":"variable",
    "name":"env",
    "location":{
      "filename":"file:///test.ts",
      "line":5,
      "col":13
    },
    "declarationKind": "export",
    "variableDef":{
      "tsType":{
        "repr":"",
        "kind":"typeLiteral",
        "typeLiteral":{
          "methods":[{
            "name":"get",
            "kind": "method",
            "params":[
              {
                "name":"key",
                "kind":"identifier",
                "optional":false,
                "tsType":{
                  "repr":"string",
                  "kind":"keyword",
                  "keyword":"string"
                }
              }
            ],
            "optional": false,
            "returnType":{
              "repr":"",
              "kind":"union",
              "union":[
                {
                  "repr":"string",
                  "kind":"keyword",
                  "keyword":"string"
                },
                {
                  "repr":"undefined",
                  "kind":"keyword",
                  "keyword":"undefined"
                }
              ]
            },
            "typeParams":[]
          }, {
            "name":"set",
            "kind": "method",
            "params":[
              {
                "name":"key",
                "kind":"identifier",
                "optional":false,
                "tsType":{
                  "repr":"string",
                  "kind":"keyword",
                  "keyword":"string"
                }
              },
              {
                "name":"value",
                "kind":"identifier",
                "optional":false,
                "tsType":{
                  "repr":"string",
                  "kind":"keyword",
                  "keyword":"string"
                }
              }
              ],
              "optional": false,
              "returnType":{
                "repr":"void",
                "kind":"keyword",
                "keyword":"void"
              },
              "typeParams":[]
            }
            ],
            "properties":[],
            "callSignatures":[],
            "indexSignatures": []
          }
        },
        "kind":"const"
      }
    },
    {
      "kind":"variable",
      "name":"num",
      "location": {
        "filename":"file:///test.ts",
        "line":13,
        "col":13
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType":{
          "repr":"100",
          "kind":"literal",
          "literal":{
            "kind":"number",
            "number":100.0
          }
        },
        "kind":"const"
      }
    },
    {
      "kind":"variable",
      "name":"bool",
      "location":{
        "filename":"file:///test.ts",
        "line":14,
        "col":13
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType":{
          "repr":"false",
          "kind":"literal",
          "literal":{
            "kind":"boolean",
            "boolean":false
          }
        },
        "kind":"const"
      }
    },
    {
      "kind":"variable",
      "name":"bigint",
      "location":{
        "filename":"file:///test.ts",
        "line":15,
        "col":13
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType":{
          "repr":"123",
          "kind":"literal",
          "literal":{
            "kind":"bigInt",
            "string":"123"
          }
        },
        "kind":"const"
      }
    },
    {
      "kind":"variable",
      "name":"regex",
      "location":{
        "filename":"file:///test.ts",
        "line":16,
        "col":13
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType":{
          "repr": "hello",
          "kind":"typeRef",
          "typeRef":{
            "typeParams":null,
            "typeName":"RegExp"
          }
        },
        "kind":"const"
      }
    },
    {
      "kind":"variable",
      "name":"date",
      "location":{
        "filename":"file:///test.ts",
        "line":17,
        "col":13
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType":{
          "repr": "Date",
          "kind":"typeRef",
          "typeRef":{
            "typeParams":null,
            "typeName":"Date"
          }
        },
        "kind":"const"
      }
    },
    {
      "kind":"variable",
      "name":"tpl1",
      "location":{
        "filename":"file:///test.ts",
        "line":18,
        "col":13
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType":{
          "repr": "foo",
          "kind":"literal",
          "literal":{
            "kind":"template",
            "tsTypes": [
              {
                "repr": "foo",
                "kind": "literal",
                "literal": {
                  "kind": "string",
                  "string": "foo"
                }
              }
            ]
          }
        },
        "kind":"const"
      }
    },
    {
      "kind":"variable",
      "name":"tpl2",
      "location":{
        "filename":"file:///test.ts",
        "line":19,
        "col":13
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType":{
          "repr": "string",
          "kind":"keyword",
          "keyword":"string"
        },
        "kind":"const"
      }
    }
    ]
  );

  json_test!(export_const_destructured,
    r#"const c = { a: "a", b: 2 };
    const d: { f: string; g: number; } = { f: "f", g: 2 };

    export const {
      /** export a doc */
      a,
      /** export b doc */
      b,
    }: {
      /** type alias doc */
      a: string;
      /** other doc */
      b: number;
    } = c;

    export const { f, g: h } = d;
    "#;
    [
      {
        "kind": "variable",
        "name": "a",
        "location": {
          "filename": "file:///test.ts",
          "line": 6,
          "col": 6,
        },
        "declarationKind": "export",
        "jsDoc": {
          "doc": "export a doc",
        },
        "variableDef": {
          "tsType": {
            "repr": "string",
            "kind": "keyword",
            "keyword": "string"
          },
          "kind": "const"
        }
      },
      {
        "kind": "variable",
        "name": "b",
        "location": {
          "filename": "file:///test.ts",
          "line": 8,
          "col": 6
        },
        "declarationKind": "export",
        "jsDoc": {
          "doc": "export b doc",
        },
        "variableDef": {
          "tsType": {
            "repr": "number",
            "kind": "keyword",
            "keyword": "number"
          },
          "kind": "const"
        }
      },
      {
        "kind": "variable",
        "name": "f",
        "location": {
          "filename": "file:///test.ts",
          "line": 16,
          "col": 19
        },
        "declarationKind": "export",
        "variableDef": {
          "tsType": {
            "repr": "string",
            "kind": "keyword",
            "keyword": "string"
          },
          "kind": "const"
        }
      },
      {
        "kind": "variable",
        "name": "h",
        "location": {
          "filename": "file:///test.ts",
          "line": 16,
          "col": 25
        },
        "declarationKind": "export",
        "variableDef": {
          "tsType": {
            "repr": "number",
            "kind": "keyword",
            "keyword": "number"
          },
          "kind": "const"
        }
      }
    ]
  );

  json_test!(export_let,
    r#"
export let str = "hello";
export let num = 100;
export let bool = false;
export let dateStr = Date();
export let regex = RegExp("foobar");
export let sym = Symbol("baz");
export let tpl = `foobarbaz`;
    "#;
    [
    {
      "kind":"variable",
      "name":"str",
      "location":{
        "filename":"file:///test.ts",
        "line":2,
        "col":11
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType": {
          "repr": "string",
          "kind": "keyword",
          "keyword":"string"
        },
        "kind":"let"
      }
    },
    {
      "kind":"variable",
      "name":"num",
      "location":{
        "filename":"file:///test.ts",
        "line":3,
        "col":11
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType": {
          "repr": "number",
          "kind": "keyword",
          "keyword":"number"
        },
        "kind":"let"
      }
    },
    {
      "kind":"variable",
      "name":"bool",
      "location":{
        "filename":"file:///test.ts",
        "line":4,
        "col":11
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType": {
          "repr": "boolean",
          "kind": "keyword",
          "keyword":"boolean"
        },
        "kind":"let"
      }
    },
    {
      "kind":"variable",
      "name":"dateStr",
      "location":{
        "filename":"file:///test.ts",
        "line":5,
        "col":11
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType": {
          "repr": "Date",
          "kind": "keyword",
          "keyword":"string"
        },
        "kind":"let"
      }
    },
    {
      "kind":"variable",
      "name":"regex",
      "location":{
        "filename":"file:///test.ts",
        "line":6,
        "col":11
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType": {
          "repr": "RegExp",
          "kind": "typeRef",
          "typeRef":{
            "typeParams":null,
            "typeName":"RegExp"
          }
        },
        "kind":"let"
      }
    },
    {
      "kind":"variable",
      "name":"sym",
      "location":{
        "filename":"file:///test.ts",
        "line":7,
        "col":11
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType": {
          "repr": "Symbol",
          "kind": "keyword",
          "keyword":"symbol"
        },
        "kind":"let"
      }
    },
    {
      "kind":"variable",
      "name":"tpl",
      "location":{
        "filename":"file:///test.ts",
        "line":8,
        "col":11
      },
      "declarationKind": "export",
      "variableDef":{
        "tsType": {
          "repr": "string",
          "kind": "keyword",
          "keyword":"string"
        },
        "kind":"let"
      }
    }
    ]
  );

  json_test!(export_class_ctor_properties,
  r#"
export class A {
  constructor(public readonly name: string, private private: number, public override public: boolean) {}
}"#;
  [{
    "kind": "class",
    "name": "A",
    "location": {
      "filename": "file:///test.ts",
      "line": 2,
      "col": 0,
    },
    "declarationKind": "export",
    "classDef": {
      "isAbstract": false,
      "constructors": [{
        "accessibility": null,
        "hasBody": true,
        "name": "constructor",
        "params": [
          {
            "accessibility": "public",
            "kind": "identifier",
            "name": "name",
            "optional": false,
            "tsType": {
              "repr": "string",
              "kind": "keyword",
              "keyword": "string",
            },
            "readonly": true,
          },
          {
            "accessibility": "private",
            "kind": "identifier",
            "name": "private",
            "optional": false,
            "tsType": {
              "repr": "number",
              "kind": "keyword",
              "keyword": "number",
            },
          },
          {
            "accessibility": "public",
            "isOverride": true,
            "kind": "identifier",
            "name": "public",
            "optional": false,
            "tsType": {
              "repr": "boolean",
              "kind": "keyword",
              "keyword": "boolean",
            }
          }
        ],
        "location": {
          "filename": "file:///test.ts",
          "line": 3,
          "col": 2,
        },
      }],
      "properties": [],
      "indexSignatures": [],
      "methods": [],
      "extends": null,
      "implements": [],
      "typeParams": [],
      "superTypeParams": [],
    }
  }]);

  json_test!(export_default_class,
    r#"
/** Class doc */
export default class Foobar {
    /** Constructor js doc */
    constructor(name: string, private private2: number, protected protected2: number) {}
}
    "#;
  [{
      "kind": "class",
      "name": "default",
      "location": {
        "filename": "file:///test.ts",
        "line": 3,
        "col": 0
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "Class doc",
      },
      "classDef": {
        "isAbstract": false,
        "extends": null,
        "implements": [],
        "typeParams": [],
        "superTypeParams": [],
        "constructors": [
          {
            "jsDoc": {
              "doc": "Constructor js doc",
            },
            "accessibility": null,
            "hasBody": true,
            "name": "constructor",
            "params": [
              {
                "name": "name",
                "kind": "identifier",
                "optional": false,
                "tsType": {
                  "repr": "string",
                  "kind": "keyword",
                  "keyword": "string"
                }
              },
              {
                "accessibility": "private",
                "name": "private2",
                "kind": "identifier",
                "optional": false,
                "tsType": {
                  "repr": "number",
                  "kind": "keyword",
                  "keyword": "number"
                }
              },
              {
                "accessibility": "protected",
                "name": "protected2",
                "kind": "identifier",
                "optional": false,
                "tsType": {
                  "repr": "number",
                  "kind": "keyword",
                  "keyword": "number"
                }
              }
            ],
            "location": {
              "filename": "file:///test.ts",
              "line": 5,
              "col": 4
            }
          }
        ],
        "properties": [],
        "indexSignatures": [],
        "methods": []
      }
  }]);

  json_test!(export_default_fn,
    r#"
export default function foo(a: number) {
  return a;
}
/** @ignore */
export default function foo(a: number) {
  return a;
}
    "#;
    [{
    "kind": "function",
    "name": "default",
    "location": {
      "filename": "file:///test.ts",
      "line": 2,
      "col": 0
    },
    "declarationKind": "export",
    "functionDef": {
      "params": [
          {
            "kind": "identifier",
            "name": "a",
            "optional": false,
            "tsType": {
              "repr": "number",
              "kind": "keyword",
              "keyword": "number",
            },
          }
      ],
      "returnType": null,
      "hasBody": true,
      "isAsync": false,
      "isGenerator": false,
      "typeParams": []
    }
  }]);

  json_test!(export_default_interface,
    r#"
/**
 * Interface js doc
 */
export default interface Reader {
    /** Read n bytes */
    read?(buf: Uint8Array, something: unknown): Promise<number>
}
    "#;
    [{
      "kind": "interface",
      "name": "default",
      "location": {
        "filename": "file:///test.ts",
        "line": 5,
        "col": 0
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "Interface js doc",
      },
      "interfaceDef": {
        "extends": [],
        "methods": [
          {
            "name": "read",
            "kind": "method",
            "location": {
              "filename": "file:///test.ts",
              "line": 7,
              "col": 4
            },
            "optional": true,
            "jsDoc": {
              "doc": "Read n bytes",
            },
            "params": [
              {
                "name": "buf",
                "kind": "identifier",
                "optional": false,
                "tsType": {
                  "repr": "Uint8Array",
                  "kind": "typeRef",
                  "typeRef": {
                    "typeParams": null,
                    "typeName": "Uint8Array"
                  }
                }
              },
              {
                "name": "something",
                "kind": "identifier",
                "optional": false,
                "tsType": {
                  "repr": "unknown",
                  "kind": "keyword",
                  "keyword": "unknown"
                }
              }
            ],
            "typeParams": [],
            "returnType": {
              "repr": "Promise",
              "kind": "typeRef",
              "typeRef": {
                "typeParams": [
                  {
                    "repr": "number",
                    "kind": "keyword",
                    "keyword": "number"
                  }
                ],
                "typeName": "Promise"
              }
            }
          }
        ],
        "properties": [],
        "callSignatures": [],
        "indexSignatures": [],
        "typeParams": []
    }
  }]);

  json_test!(export_default_expr,
    r#"export default "foo";"#;
    [
      {
        "kind": "variable",
        "name": "default",
        "location": {
          "filename": "file:///test.ts",
          "line": 1,
          "col": 0
        },
        "declarationKind": "export",
        "variableDef": {
          "tsType": {
            "repr": "foo",
            "kind": "literal",
            "literal": {
              "kind": "string",
              "string": "foo"
            }
          },
          "kind": "var"
        }
      }
    ]
  );

  json_test!(export_enum,
    r#"
/**
 * Some enum for good measure
 */
export enum Hello {
    World = "world",
    /** There is a JsDoc */
    Fizz = "fizz",
    /** There is a JsDoc
     * @ignore
     */
    Buzz = "buzz",
}
    "#;
    [{
    "kind": "enum",
    "name": "Hello",
    "location": {
      "filename": "file:///test.ts",
      "line": 5,
      "col": 0
    },
    "declarationKind": "export",
    "jsDoc": {
      "doc": "Some enum for good measure",
    },
    "enumDef": {
      "members": [
        {
          "name": "World",
          "init": {
            "repr": "world",
            "kind": "literal",
            "literal": {
              "kind": "string",
              "string": "world",
            }
          },
          "location": {
            "filename": "file:///test.ts",
            "line": 6,
            "col": 4,
          }
        },
        {
          "name": "Fizz",
          "init": {
            "repr": "fizz",
            "kind": "literal",
            "literal": {
              "kind": "string",
              "string": "fizz",
            }
          },
          "jsDoc": {
            "doc": "There is a JsDoc"
          },
          "location": {
            "filename": "file:///test.ts",
            "line": 8,
            "col": 4,
          }
        }
      ]
    }
  }]);

  json_test!(export_fn,
    r#"/**
* @module foo
*/

/**
* Hello there, this is a multiline JSdoc.
*
* It has many lines
*
* Or not that many?
*/
export function foo(a: string, b?: number, cb: (...cbArgs: unknown[]) => void, ...args: unknown[]): void {
    /**
     * @todo document all the things.
     */
    console.log("Hello world");
}
    "#;
  [
    {
      "kind": "moduleDoc",
      "name": "",
      "location": {
        "filename": "file:///test.ts",
        "line": 1,
        "col": 0,
      },
      "declarationKind": "export",
      "jsDoc": {
        "tags": [
          {
            "kind": "module"
          }
        ]
      },
    }, {
      "functionDef": {
        "hasBody": true,
        "isAsync": false,
        "isGenerator": false,
        "typeParams": [],
        "params": [
            {
              "name": "a",
              "kind": "identifier",
              "optional": false,
              "tsType": {
                "keyword": "string",
                "kind": "keyword",
                "repr": "string",
              },
            },
            {
              "name": "b",
              "kind": "identifier",
              "optional": true,
              "tsType": {
                "keyword": "number",
                "kind": "keyword",
                "repr": "number",
              },
            },
            {
              "name": "cb",
              "kind": "identifier",
              "optional": false,
              "tsType": {
                "repr": "",
                "kind": "fnOrConstructor",
                "fnOrConstructor": {
                  "constructor": false,
                  "tsType": {
                    "keyword": "void",
                    "kind": "keyword",
                    "repr": "void"
                  },
                  "typeParams": [],
                  "params": [{
                    "arg": {
                      "name": "cbArgs",
                      "kind": "identifier",
                      "optional": false,
                      "tsType": null
                    },
                    "kind": "rest",
                    "tsType": {
                      "repr": "",
                      "kind": "array",
                      "array": {
                          "repr": "unknown",
                          "kind": "keyword",
                          "keyword": "unknown"
                      }
                    },
                  }]
                }
              },
            },
            {
              "arg": {
                "name": "args",
                "kind": "identifier",
                "optional": false,
                "tsType": null
              },
              "kind": "rest",
              "tsType": {
                "array": {
                  "keyword": "unknown",
                  "kind": "keyword",
                  "repr": "unknown"
                },
                "kind": "array",
                "repr": ""
              }
            }
        ],
        "returnType": {
          "keyword": "void",
          "kind": "keyword",
          "repr": "void",
        },
      },
      "declarationKind": "export",
      "jsDoc": {
        "doc": "Hello there, this is a multiline JSdoc.\n\nIt has many lines\n\nOr not that many?",
      },
      "kind": "function",
      "location": {
        "col": 0,
        "filename": "file:///test.ts",
        "line": 12,
      },
      "name": "foo",
    }]);

  json_test!(export_fn2,
    r#"
interface AssignOpts {
  a: string;
}

export function foo([e,,f, ...g]: number[], { c, d: asdf, i = "asdf", ...rest}, ops: AssignOpts = {}): void {
    console.log("Hello world");
}
    "#,
    false;
  [{
    "functionDef": {
      "hasBody": true,
      "isAsync": false,
      "isGenerator": false,
      "typeParams": [],
      "params": [
        {
          "elements": [
            {
              "name": "e",
              "kind": "identifier",
              "optional": false,
              "tsType": null
            },
            null,
            {
              "name": "f",
              "kind": "identifier",
              "optional": false,
              "tsType": null
            },
            {
              "arg": {
                "name": "g",
                "kind": "identifier",
                "optional": false,
                "tsType": null
              },
              "kind": "rest",
              "tsType": null
            }
          ],
          "kind": "array",
          "optional": false,
          "tsType": {
            "repr": "",
            "kind": "array",
            "array": {
                "repr": "number",
                "kind": "keyword",
                "keyword": "number"
            }
          }
        },
        {
          "kind": "object",
          "optional": false,
          "props": [
            {
              "kind": "assign",
              "key": "c",
              "value": null
            },
            {
              "kind": "keyValue",
              "key": "d",
              "value": {
                "name": "asdf",
                "kind": "identifier",
                "optional": false,
                "tsType": null
              }
            },
            {
              "kind": "assign",
              "key": "i",
              "value": "[UNSUPPORTED]"
            },
            {
              "arg": {
                "name": "rest",
                "kind": "identifier",
                "optional": false,
                "tsType": null
              },
              "kind": "rest"
            }
          ],
          "tsType": null
        },
        {
          "kind": "assign",
          "left": {
            "name": "ops",
            "kind": "identifier",
            "optional": false,
            "tsType": {
              "repr": "AssignOpts",
              "kind": "typeRef",
              "typeRef": {
                "typeName": "AssignOpts",
                "typeParams": null,
              }
            }
          },
          "right": "[UNSUPPORTED]",
          "tsType": null
        }
      ],
      "returnType": {
        "keyword": "void",
        "kind": "keyword",
        "repr": "void",
      },
    },
    "kind": "function",
    "declarationKind": "export",
    "location": {
      "col": 0,
      "filename": "file:///test.ts",
      "line": 6,
    },
    "name": "foo",
  }, {
    "kind": "interface",
    "name": "AssignOpts",
    "location": {
      "col": 0,
      "filename": "file:///test.ts",
      "line": 2,
    },
    "declarationKind": "private",
    "interfaceDef": {
      "extends": [],
      "methods": [],
      "properties": [{
        "name": "a",
        "location": {
            "filename": "file:///test.ts",
            "line": 3,
            "col": 2,
        },
        "params": [],
        "computed": false,
        "optional": false,
        "tsType": {
          "repr": "string",
          "kind": "keyword",
          "keyword": "string",
        },
        "typeParams": [],
      }],
      "callSignatures": [],
      "indexSignatures": [],
      "typeParams": [],
    }
  }], vec!["file:///test.ts:2:0 PrivateTypeRef"]);

  json_test!(export_interface,
        r#"
interface Foo {
}
interface Bar {
}
/**
 * Interface js doc
 */
export interface Reader extends Foo, Bar {
    /** Read n bytes */
    read?(buf: Uint8Array, something: unknown): Promise<number>
}
    "#,
      false;
      [{
          "kind": "interface",
          "name": "Reader",
          "location": {
            "filename": "file:///test.ts",
            "line": 9,
            "col": 0
          },
          "declarationKind": "export",
          "jsDoc": {
            "doc": "Interface js doc",
          },
          "interfaceDef": {
            "extends": [
              {
                "repr": "Foo",
                "kind": "typeRef",
                "typeRef": {
                  "typeParams": null,
                  "typeName": "Foo"
                }
              },
              {
                "repr": "Bar",
                "kind": "typeRef",
                "typeRef": {
                  "typeParams": null,
                  "typeName": "Bar"
                }
              }
            ],
            "methods": [
              {
                "name": "read",
                "kind": "method",
                "location": {
                  "filename": "file:///test.ts",
                  "line": 11,
                  "col": 4
                },
                "optional": true,
                "jsDoc": {
                  "doc": "Read n bytes",
                },
                "params": [
                  {
                    "name": "buf",
                    "kind": "identifier",
                    "optional": false,
                    "tsType": {
                      "repr": "Uint8Array",
                      "kind": "typeRef",
                      "typeRef": {
                        "typeParams": null,
                        "typeName": "Uint8Array"
                      }
                    }
                  },
                  {
                    "name": "something",
                    "kind": "identifier",
                    "optional": false,
                    "tsType": {
                      "repr": "unknown",
                      "kind": "keyword",
                      "keyword": "unknown"
                    }
                  }
                ],
                "typeParams": [],
                "returnType": {
                  "repr": "Promise",
                  "kind": "typeRef",
                  "typeRef": {
                    "typeParams": [
                      {
                        "repr": "number",
                        "kind": "keyword",
                        "keyword": "number"
                      }
                    ],
                    "typeName": "Promise"
                  }
                }
              }
            ],
            "properties": [],
            "callSignatures": [],
            "indexSignatures": [],
            "typeParams": [],
        }
      }, {
        "kind": "interface",
        "name": "Foo",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 0
        },
        "declarationKind": "private",
        "interfaceDef": {
          "extends": [],
          "methods": [],
          "properties": [],
          "callSignatures": [],
          "indexSignatures": [],
          "typeParams": [],
      }
    },  {
      "kind": "interface",
      "name": "Bar",
      "location": {
        "filename": "file:///test.ts",
        "line": 4,
        "col": 0
      },
      "declarationKind": "private",
      "interfaceDef": {
        "extends": [],
        "methods": [],
        "properties": [],
        "callSignatures": [],
        "indexSignatures": [],
        "typeParams": [],
    }
  }], vec![
    "file:///test.ts:2:0 PrivateTypeRef",
    "file:///test.ts:4:0 PrivateTypeRef"
  ]);

  json_test!(export_interface2,
    r#"
export interface TypedIface<T> {
    something(): T
}
    "#;
    [{
      "kind": "interface",
      "name": "TypedIface",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 0
      },
      "declarationKind": "export",
      "interfaceDef": {
        "extends": [],
        "methods": [
          {
            "name": "something",
            "kind": "method",
            "location": {
              "filename": "file:///test.ts",
              "line": 3,
              "col": 4
            },
            "optional": false,
            "params": [],
            "typeParams": [],
            "returnType": {
              "repr": "T",
              "kind": "typeRef",
              "typeRef": {
                "typeParams": null,
                "typeName": "T"
              }
            }
          }
        ],
        "properties": [],
        "callSignatures": [],
        "indexSignatures": [],
        "typeParams": [
          { "name": "T" }
        ],
    }
  }]);

  json_test!(export_interface_accessors,
    r#"
export interface Thing {
  get size(): number;
  set size(value: number | string);
}
    "#;
    [{
      "kind": "interface",
      "name": "Thing",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 0
      },
      "declarationKind": "export",
      "interfaceDef": {
        "extends": [],
        "methods": [
          {
            "name": "size",
            "kind": "getter",
            "location": {
              "filename": "file:///test.ts",
              "line": 3,
              "col": 2,
            },
            "optional": false,
            "params": [],
            "typeParams": [],
            "returnType": {
              "repr": "number",
              "kind": "keyword",
              "keyword": "number",
            },
          },
          {
            "name": "size",
            "kind": "setter",
            "location": {
              "filename": "file:///test.ts",
              "line": 4,
              "col": 2,
            },
            "optional": false,
            "params": [
              {
                "kind": "identifier",
                "name": "value",
                "optional": false,
                "tsType": {
                  "repr": "",
                  "kind": "union",
                  "union": [
                    {
                      "repr": "number",
                      "kind": "keyword",
                      "keyword": "number",
                    },
                    {
                      "repr": "string",
                      "kind": "keyword",
                      "keyword": "string",
                    }
                  ]
                }
              }
            ],
            "typeParams": [],
            "returnType": null,
          },
        ],
        "properties": [],
        "callSignatures": [],
        "indexSignatures": [],
        "typeParams": [],
      }
    }]
  );

  json_test!(export_type_alias,
    r#"
/** Array holding numbers */
export type NumberArray = Array<number>;
    "#;
    [{
    "kind": "typeAlias",
    "name": "NumberArray",
    "location": {
        "filename": "file:///test.ts",
      "line": 3,
      "col": 0
    },
    "declarationKind": "export",
    "jsDoc": {
      "doc": "Array holding numbers",
    },
    "typeAliasDef": {
      "typeParams": [],
      "tsType": {
        "repr": "Array",
        "kind": "typeRef",
        "typeRef": {
          "typeParams": [
            {
              "repr": "number",
              "kind": "keyword",
              "keyword": "number"
            }
          ],
          "typeName": "Array"
        }
      }
    }
  }]);

  json_test!(export_type_alias_literal,
  r#"
export type A = {
  new (d: string): A;
  a(): void;
  b?(): void;
  get c(): string;
  set c(v: number);
};
"#;
  [{
    "kind": "typeAlias",
    "name": "A",
    "location": {
      "filename": "file:///test.ts",
      "line": 2,
      "col": 0,
    },
    "declarationKind": "export",
    "typeAliasDef": {
      "typeParams": [],
      "tsType": {
        "repr": "",
        "kind": "typeLiteral",
        "typeLiteral": {
          "methods": [
            {
              "name": "new",
              "kind": "method",
              "params": [
                {
                  "kind": "identifier",
                  "name": "d",
                  "optional": false,
                  "tsType": {
                    "repr": "string",
                    "kind": "keyword",
                    "keyword": "string",
                  }
                }
              ],
              "optional": false,
              "returnType": {
                "repr": "A",
                "kind": "typeRef",
                "typeRef": {
                  "typeParams": null,
                  "typeName": "A",
                }
              },
              "typeParams": [],
            },
            {
              "name": "a",
              "kind": "method",
              "params": [],
              "optional": false,
              "returnType": {
                "repr": "void",
                "kind": "keyword",
                "keyword": "void",
              },
              "typeParams": [],
            },
            {
              "name": "b",
              "kind": "method",
              "params": [],
              "optional": true,
              "returnType": {
                "repr": "void",
                "kind": "keyword",
                "keyword": "void",
              },
              "typeParams": [],
            },
            {
              "name": "c",
              "kind": "getter",
              "params": [],
              "optional": false,
              "returnType": {
                "repr": "string",
                "kind": "keyword",
                "keyword": "string",
              },
              "typeParams": [],
            },
            {
              "name": "c",
              "kind": "setter",
              "params": [
                {
                  "kind": "identifier",
                  "name": "v",
                  "optional": false,
                  "tsType": {
                    "repr": "number",
                    "kind": "keyword",
                    "keyword": "number",
                  }
                }
              ],
              "optional": false,
              "returnType": null,
              "typeParams": [],
            }
          ],
          "properties": [],
          "callSignatures": [],
          "indexSignatures": []
        }
      }
    }
  }]);

  json_test!(export_namespace,
    r#"
/** Namespace JSdoc */
export namespace RootNs {
    export const a = "a";

    /** Nested namespace JSDoc */
    export namespace NestedNs {
      export enum Foo {
        a = 1,
        b = 2,
        c = 3,
      }
    }
}

export namespace RootNs.OtherNs {
  export class Other {}
}
    "#;
    [{
    "kind": "namespace",
    "name": "RootNs",
    "location": {
      "filename": "file:///test.ts",
      "line": 3,
      "col": 0
    },
    "declarationKind": "export",
    "jsDoc": {
      "doc": "Namespace JSdoc",
    },
    "namespaceDef": {
      "elements": [
        {
          "kind": "variable",
          "name": "a",
          "location": {
            "filename": "file:///test.ts",
            "line": 4,
            "col": 17
          },
          "declarationKind": "export",
          "variableDef": {
            "tsType": {
              "repr": "a",
              "kind": "literal",
              "literal": {
                "kind": "string",
                "string": "a"
              }
            },
            "kind": "const"
          }
        },
        {
          "kind": "namespace",
          "name": "NestedNs",
          "location": {
            "filename": "file:///test.ts",
            "line": 7,
            "col": 4
          },
          "declarationKind": "export",
          "jsDoc": {
            "doc": "Nested namespace JSDoc",
          },
          "namespaceDef": {
            "elements": [
              {
                "kind": "enum",
                "name": "Foo",
                "location": {
                  "filename": "file:///test.ts",
                  "line": 8,
                  "col": 6
                },
                "declarationKind": "export",
                "enumDef": {
                  "members": [
                    {
                      "name": "a",
                      "init": {
                        "repr": "1",
                        "kind": "literal",
                        "literal": {
                          "kind": "number",
                          "number": 1.0,
                        }
                      },
                      "location": {
                        "filename": "file:///test.ts",
                        "line": 9,
                        "col": 8,
                      }
                    },
                    {
                      "name": "b",
                      "init": {
                        "repr": "2",
                        "kind": "literal",
                        "literal": {
                          "kind": "number",
                          "number": 2.0,
                        }
                      },
                      "location": {
                        "filename": "file:///test.ts",
                        "line": 10,
                        "col": 8,
                      }
                    },
                    {
                      "name": "c",
                      "init": {
                        "repr": "3",
                        "kind": "literal",
                        "literal": {
                          "kind": "number",
                          "number": 3.0,
                        }
                      },
                      "location": {
                        "filename": "file:///test.ts",
                        "line": 11,
                        "col": 8,
                      }
                    }
                  ]
                }
              }
            ]
          }
        },
        {
          "kind": "namespace",
          "name": "OtherNs",
          "location": {
            "filename": "file:///test.ts",
            "line": 16,
            "col": 7
          },
          "declarationKind": "export",
          "namespaceDef": {
            "elements": [
              {
                "kind": "class",
                "name": "Other",
                "location": {
                  "filename": "file:///test.ts",
                  "line": 17,
                  "col": 2
                },
                "declarationKind": "export",
                "classDef": {
                  "isAbstract": false,
                  "constructors": [],
                  "properties": [],
                  "indexSignatures": [],
                  "methods": [],
                  "extends": null,
                  "implements": [],
                  "typeParams": [],
                  "superTypeParams": [],
                }
              }
            ]
          }
        }
      ]
    }
  }]);

  json_test!(export_namespace_enum_same_name,
    r#"
export namespace RootNs {
  export namespace NestedNs {
    export enum Foo {
    }
  }

  export enum Foo {
  }
}
    "#;
    [{
    "kind": "namespace",
    "name": "RootNs",
    "location": {
      "filename": "file:///test.ts",
      "line": 2,
      "col": 0
    },
    "declarationKind": "export",
    "namespaceDef": {
      "elements": [
        {
          "kind": "namespace",
          "name": "NestedNs",
          "location": {
            "filename": "file:///test.ts",
            "line": 3,
            "col": 2
          },
          "declarationKind": "export",
          "namespaceDef": {
            "elements": [
              {
                "kind": "enum",
                "name": "Foo",
                "location": {
                  "filename": "file:///test.ts",
                  "line": 4,
                  "col": 4
                },
                "declarationKind": "export",
                "enumDef": {
                  "members": []
                }
              }
            ]
          }
        },
        {
          "kind": "enum",
          "name": "Foo",
          "location": {
            "filename": "file:///test.ts",
            "line": 8,
            "col": 2
          },
          "declarationKind": "export",
          "enumDef": {
            "members": []
          }
        }
      ]
    }
  }]);

  json_test!(export_declaration_merged_namespace,
    r#"
namespace Namespace1 {
  export class Test1 {}
}
namespace Namespace1 {
  export class Test2 {}
}

export { Namespace1 };
"#;
    [{
      "kind": "namespace",
      "name": "Namespace1",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 0,
      },
      "declarationKind": "export",
      "namespaceDef": {
        "elements": [{
          "kind": "class",
          "name": "Test1",
          "location": {
            "filename": "file:///test.ts",
            "line": 3,
            "col": 2,
          },
          "declarationKind": "export",
          "classDef": {
            "isAbstract": false,
            "constructors": [],
            "properties": [],
            "indexSignatures": [],
            "methods": [],
            "extends": null,
            "implements": [],
            "typeParams": [],
            "superTypeParams": []
          }
        }, {
          "kind": "class",
          "name": "Test2",
          "location": {
            "filename": "file:///test.ts",
            "line": 6,
            "col": 2,
          },
          "declarationKind": "export",
          "classDef": {
            "isAbstract": false,
            "constructors": [],
            "properties": [],
            "indexSignatures": [],
            "methods": [],
            "extends": null,
            "implements": [],
            "typeParams": [],
            "superTypeParams": []
          }
        }]
      }
    }]
  );

  json_test!(exports_declared_earlier,
      r#"
const hello = "world";
function say(words: string): void { }
function foo(): void { }
export { hello, say, foo as bar };
    "#;
  [
    {
      "kind": "variable",
      "name": "hello",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 6
      },
      "declarationKind": "export",
      "variableDef": {
        "tsType": {
          "repr": "world",
          "kind": "literal",
          "literal": {
            "kind": "string",
            "string": "world"
          }
        },
        "kind": "const"
      }
    },
    {
      "kind": "function",
      "name": "say",
      "location": {
        "filename": "file:///test.ts",
        "line": 3,
        "col": 0
      },
      "declarationKind": "export",
      "functionDef": {
        "params": [
          {
            "kind": "identifier",
            "name": "words",
            "optional": false,
            "tsType": {
              "repr": "string",
              "kind": "keyword",
              "keyword": "string"
            }
          }
        ],
        "returnType": {
          "repr": "void",
          "kind": "keyword",
          "keyword": "void"
        },
        "hasBody": true,
        "isAsync": false,
        "isGenerator": false,
        "typeParams": []
      }
    },
    {
      "kind": "function",
      "name": "bar",
      "location": {
        "filename": "file:///test.ts",
        "line": 4,
        "col": 0
      },
      "declarationKind": "export",
      "functionDef": {
        "params": [],
        "returnType": {
          "repr": "void",
          "kind": "keyword",
          "keyword": "void"
        },
        "hasBody": true,
        "isAsync": false,
        "isGenerator": false,
        "typeParams": []
      }
    }
  ]
    );

  json_test!(non_implemented_renamed_exports_declared_earlier,
    r#"
  declare function foo(): void;
  export { foo as bar };
    "#;
    [
      {
        "kind": "function",
        "name": "bar",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 2
        },
        "declarationKind": "export",
        "functionDef": {
          "params": [],
          "returnType": {
            "repr": "void",
            "kind": "keyword",
            "keyword": "void"
          },
          "isAsync": false,
          "isGenerator": false,
          "typeParams": []
        }
      }
    ]
  );

  json_test!(no_ambient_in_module,
    r#"
declare function foo(): number;
export function bar() {};
    "#;
    [
      {
        "kind": "function",
        "name": "bar",
        "location": {
          "filename": "file:///test.ts",
          "line": 3,
          "col": 0
        },
        "declarationKind": "export",
        "functionDef": {
          "params": [],
          "returnType": null,
          "hasBody": true,
          "isAsync": false,
          "isGenerator": false,
          "typeParams": []
        }
      }
    ]
  );

  json_test!(default_exports_declared_earlier,
    r#"
function foo(): void {}
export default foo;
    "#;
    [
      {
        "kind": "function",
        "name": "default",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 0
        },
        "declarationKind": "export",
        "functionDef": {
          "params": [],
          "returnType": {
            "repr": "void",
            "kind": "keyword",
            "keyword": "void"
          },
          "hasBody": true,
          "isAsync": false,
          "isGenerator": false,
          "typeParams": []
        }
      }
    ]
  );

  json_test!(reexport_existing_export,
    r#"
export function foo(): void {}
export { foo as bar };
    "#;
    [
      {
        "kind": "function",
        "name": "foo",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 0
        },
        "declarationKind": "export",
        "functionDef": {
          "params": [],
          "returnType": {
            "repr": "void",
            "kind": "keyword",
            "keyword": "void"
          },
          "hasBody": true,
          "isAsync": false,
          "isGenerator": false,
          "typeParams": []
        }
      },
      {
        "kind": "function",
        "name": "bar",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 0
        },
        "declarationKind": "export",
        "functionDef": {
          "params": [],
          "returnType": {
            "repr": "void",
            "kind": "keyword",
            "keyword": "void"
          },
          "hasBody": true,
          "isAsync": false,
          "isGenerator": false,
          "typeParams": []
        }
      }
    ]
  );

  json_test!(optional_return_type,
    r#"
  export function foo(a: number) {
    return a;
  }
    "#;
    [{
      "kind": "function",
      "name": "foo",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 2
      },
      "declarationKind": "export",
      "functionDef": {
        "params": [
            {
              "name": "a",
              "kind": "identifier",
              "optional": false,
              "tsType": {
                "keyword": "number",
                "kind": "keyword",
                "repr": "number",
              },
            }
        ],
        "typeParams": [],
        "returnType": null,
        "hasBody": true,
        "isAsync": false,
        "isGenerator": false
      }
    }]
  );

  json_test!(ts_lit_types,
    r#"
export type boolLit = false;
export type strLit = "text";
export type tplLit = `text`;
export type tplLitArg = `test${number}`;
export type numLit = 5;
    "#;
  [
    {
      "kind": "typeAlias",
      "name": "boolLit",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 0
      },
      "declarationKind": "export",
      "typeAliasDef": {
        "tsType": {
          "repr": "false",
          "kind": "literal",
          "literal": {
            "kind": "boolean",
            "boolean": false
          }
        },
        "typeParams": []
      }
    }, {
      "kind": "typeAlias",
      "name": "strLit",
      "location": {
        "filename": "file:///test.ts",
        "line": 3,
        "col": 0
      },
      "declarationKind": "export",
      "typeAliasDef": {
        "tsType": {
          "repr": "text",
          "kind": "literal",
          "literal": {
            "kind": "string",
            "string": "text"
          }
        },
        "typeParams": []
      }
    }, {
      "kind": "typeAlias",
      "name": "tplLit",
      "location": {
        "filename": "file:///test.ts",
        "line": 4,
        "col": 0
      },
      "declarationKind": "export",
      "typeAliasDef": {
        "tsType": {
          "repr": "text",
          "kind": "literal",
          "literal": {
            "kind": "template",
            "tsTypes": [
              {
                "repr": "text",
                "kind": "literal",
                "literal": {
                  "kind": "string",
                  "string": "text"
                }
              }
            ]
          }
        },
        "typeParams": []
      }
    }, {
      "kind": "typeAlias",
      "name": "tplLitArg",
      "location": {
        "filename": "file:///test.ts",
        "line": 5,
        "col": 0,
      },
      "declarationKind": "export",
      "typeAliasDef": {
        "tsType": {
          "repr": "test${number}",
          "kind": "literal",
          "literal": {
            "kind": "template",
            "tsTypes": [
              {
                "repr": "test",
                "kind": "literal",
                "literal": {
                  "kind": "string",
                  "string": "test",
                }
              }, {
                "repr": "number",
                "kind": "keyword",
                "keyword": "number"
              }, {
                "repr": "",
                "kind": "literal",
                "literal": {
                  "kind": "string",
                  "string": ""
                }
              }
            ]
          }
        },
        "typeParams": [],
      }
    }, {
      "kind": "typeAlias",
      "name": "numLit",
      "location": {
        "filename": "file:///test.ts",
        "line": 6,
        "col": 0
      },
      "declarationKind": "export",
      "typeAliasDef": {
        "tsType": {
          "repr": "5",
          "kind": "literal",
          "literal": {
            "kind": "number",
            "number": 5.0
          }
        },
        "typeParams": []
      }
    }
  ]);

  json_test!(export_private,
    r#"
const foo: string = "foo";
export { foo };
    "#,
    private;
    [
      {
        "kind": "variable",
        "name": "foo",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 6
        },
        "declarationKind": "export",
        "variableDef": {
          "tsType": {
            "repr": "string",
            "kind": "keyword",
            "keyword": "string"
          },
          "kind": "const"
        }
      }
    ]
  );

  json_test!(ts_type_predicate_1,
    r#"
export function foo(bar: A | B): bar is A {}
    "#;
    [
      {
        "kind": "function",
        "name": "foo",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 0
        },
        "declarationKind": "export",
        "functionDef": {
          "params": [
            {
              "kind": "identifier",
              "name": "bar",
              "optional": false,
              "tsType": {
                "repr": "",
                "kind": "union",
                "union": [
                  {
                    "repr": "A",
                    "kind": "typeRef",
                    "typeRef": {
                      "typeParams": null,
                      "typeName": "A"
                    }
                  },
                  {
                    "repr": "B",
                    "kind": "typeRef",
                    "typeRef": {
                      "typeParams": null,
                      "typeName": "B"
                    }
                  }
                ]
              }
            }
          ],
          "returnType": {
            "repr": "bar is A",
            "kind": "typePredicate",
            "typePredicate": {
              "asserts": false,
              "param": {
                "type": "identifier",
                "name": "bar"
              },
              "type": {
                "repr": "A",
                "kind": "typeRef",
                "typeRef": {
                  "typeParams": null,
                  "typeName": "A"
                }
              }
            }
          },
          "hasBody": true,
          "isAsync": false,
          "isGenerator": false,
          "typeParams": []
        }
      }
    ]
  );

  json_test!(ts_type_predicate_2,
    r#"
export function foo(bar: A | B): asserts bar is B {}
    "#;
    [
      {
        "kind": "function",
        "name": "foo",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 0
        },
        "declarationKind": "export",
        "functionDef": {
          "params": [
            {
              "kind": "identifier",
              "name": "bar",
              "optional": false,
              "tsType": {
                "repr": "",
                "kind": "union",
                "union": [
                  {
                    "repr": "A",
                    "kind": "typeRef",
                    "typeRef": {
                      "typeParams": null,
                      "typeName": "A"
                    }
                  },
                  {
                    "repr": "B",
                    "kind": "typeRef",
                    "typeRef": {
                      "typeParams": null,
                      "typeName": "B"
                    }
                  }
                ]
              }
            }
          ],
          "returnType": {
            "repr": "asserts bar is B",
            "kind": "typePredicate",
            "typePredicate": {
              "asserts": true,
              "param": {
                "type": "identifier",
                "name": "bar"
              },
              "type": {
                "repr": "B",
                "kind": "typeRef",
                "typeRef": {
                  "typeParams": null,
                  "typeName": "B"
                }
              }
            }
          },
          "hasBody": true,
          "isAsync": false,
          "isGenerator": false,
          "typeParams": []
        }
      }
    ]
  );

  json_test!(ts_type_predicate_3,
    r#"
export class C {
  isSomething(): this is Something {}
}
    "#;
    [
      {
        "kind": "class",
        "name": "C",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 0
        },
        "declarationKind": "export",
        "classDef": {
          "isAbstract": false,
          "constructors": [],
          "properties": [],
          "indexSignatures": [],
          "methods": [
            {
              "accessibility": null,
              "optional": false,
              "isAbstract": false,
              "isStatic": false,
              "name": "isSomething",
              "kind": "method",
              "functionDef": {
                "params": [],
                "returnType": {
                  "repr": "this is Something",
                  "kind": "typePredicate",
                  "typePredicate": {
                    "asserts": false,
                    "param": {
                      "type": "this"
                    },
                    "type": {
                      "repr": "Something",
                      "kind": "typeRef",
                      "typeRef": {
                        "typeParams": null,
                        "typeName": "Something",
                      },
                    },
                  },
                },
                "hasBody": true,
                "isAsync": false,
                "isGenerator": false,
                "typeParams": [],
              },
              "location": {
                "filename": "file:///test.ts",
                "line": 3,
                "col": 2
              }
            }
          ],
          "extends": null,
          "implements": [],
          "typeParams": [],
          "superTypeParams": []
        }
      }
    ]
  );

  json_test!(ts_type_assertion,
    r#"
export function foo(bar: any): asserts bar {}
    "#;
    [
      {
        "kind": "function",
        "name": "foo",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 0
        },
        "declarationKind": "export",
        "functionDef": {
          "params": [
            {
              "kind": "identifier",
              "name": "bar",
              "optional": false,
              "tsType": {
                "repr": "any",
                "kind": "keyword",
                "keyword": "any"
              }
            }
          ],
          "returnType": {
            "repr": "asserts bar",
            "kind": "typePredicate",
            "typePredicate": {
              "asserts": true,
              "param": {
                "type": "identifier",
                "name": "bar"
              },
              "type": null,
            }
          },
          "hasBody": true,
          "isAsync": false,
          "isGenerator": false,
          "typeParams": []
        }
      }
    ]
  );

  json_test!(infer_types,
  r#"export type Flatten<T> = T extends Array<infer U> ? U : T;"#;
  [
    {
      "kind": "typeAlias",
      "name": "Flatten",
      "location": {
        "filename": "file:///test.ts",
        "line": 1,
        "col": 0
      },
      "declarationKind": "export",
      "typeAliasDef": {
        "tsType": {
          "repr": "",
          "kind": "conditional",
          "conditionalType": {
            "checkType": {
              "repr": "T",
              "kind": "typeRef",
              "typeRef": {
                "typeParams": null,
                "typeName": "T"
              }
            },
            "extendsType": {
              "repr": "Array",
              "kind": "typeRef",
              "typeRef": {
                "typeParams": [
                  {
                    "repr": "",
                    "kind": "infer",
                    "infer": {
                      "typeParam": {
                        "name": "U"
                      }
                    }
                  }
                ],
                "typeName": "Array"
              }
            },
            "trueType": {
              "repr": "U",
              "kind": "typeRef",
              "typeRef": {
                "typeParams": null,
                "typeName": "U"
              }
            },
            "falseType": {
              "repr": "T",
              "kind": "typeRef",
              "typeRef": {
                "typeParams": null,
                "typeName": "T",
              }
            }
          }
        },
        "typeParams": [
          {
            "name": "T"
          }
        ]
      }
    }
  ]);

  json_test!(infer_object_literal,
    r#"
    const s: symbol = Symbol.for("s");
    const t: symbol = Symbol.for("t");

    export const a = {
      a: "a",
      b: new Map<string, number>(),
      c: { d: "d" },
      d(e: string): void {},
      f: (g: string): void => {},
      get h(): string {
        return "h";
      },
      set h(value: string) {

      },
      [s]: [1, 2, 3, "a"],
      [t](u: string): void {},
    };"#;
    [
      {
        "kind": "variable",
        "name": "a",
        "location": {
          "filename": "file:///test.ts",
          "line": 5,
          "col": 17
        },
        "declarationKind": "export",
        "variableDef": {
          "tsType": {
            "repr": "",
            "kind": "typeLiteral",
            "typeLiteral": {
              "methods": [
                {
                  "name": "d",
                  "kind": "method",
                  "params": [
                    {
                      "kind": "identifier",
                      "name": "e",
                      "optional": false,
                      "tsType": {
                        "repr": "string",
                        "kind": "keyword",
                        "keyword": "string"
                      }
                    }
                  ],
                  "optional": false,
                  "returnType": {
                    "repr": "void",
                    "kind": "keyword",
                    "keyword": "void"
                  },
                  "typeParams": []
                },
                {
                  "name": "h",
                  "kind": "getter",
                  "params": [],
                  "optional": false,
                  "returnType": {
                    "repr": "string",
                    "kind": "keyword",
                    "keyword": "string"
                  },
                  "typeParams": []
                },
                {
                  "name": "h",
                  "kind": "setter",
                  "params": [
                    {
                      "kind": "identifier",
                      "name": "value",
                      "optional": false,
                      "tsType": {
                        "repr": "string",
                        "kind": "keyword",
                        "keyword": "string"
                      }
                    }
                  ],
                  "optional": false,
                  "returnType": null,
                  "typeParams": []
                },
                {
                  "name": "[t]",
                  "kind": "method",
                  "params": [
                    {
                      "kind": "identifier",
                      "name": "u",
                      "optional": false,
                      "tsType": {
                        "repr": "string",
                        "kind": "keyword",
                        "keyword": "string"
                      }
                    }
                  ],
                  "computed": true,
                  "optional": false,
                  "returnType": {
                    "repr": "void",
                    "kind": "keyword",
                    "keyword": "void"
                  },
                  "typeParams": []
                }
              ],
              "properties": [
                {
                  "name": "a",
                  "params": [],
                  "computed": false,
                  "optional": false,
                  "tsType": {
                    "repr": "string",
                    "kind": "keyword",
                    "keyword": "string"
                  },
                  "typeParams": []
                },
                {
                  "name": "b",
                  "params": [],
                  "computed": false,
                  "optional": false,
                  "tsType": {
                    "repr": "Map",
                    "kind": "typeRef",
                    "typeRef": {
                      "typeParams": [
                        {
                          "repr": "string",
                          "kind": "keyword",
                          "keyword": "string"
                        },
                        {
                          "repr": "number",
                          "kind": "keyword",
                          "keyword": "number"
                        }
                      ],
                      "typeName": "Map"
                    }
                  },
                  "typeParams": []
                },
                {
                  "name": "c",
                  "params": [],
                  "computed": false,
                  "optional": false,
                  "tsType": {
                    "repr": "",
                    "kind": "typeLiteral",
                    "typeLiteral": {
                      "methods": [],
                      "properties": [
                        {
                          "name": "d",
                          "params": [],
                          "computed": false,
                          "optional": false,
                          "tsType": {
                            "repr": "string",
                            "kind": "keyword",
                            "keyword": "string"
                          },
                          "typeParams": []
                        }
                      ],
                      "callSignatures": [],
                      "indexSignatures": []
                    }
                  },
                  "typeParams": []
                },
                {
                  "name": "f",
                  "params": [],
                  "computed": false,
                  "optional": false,
                  "tsType": {
                    "repr": "",
                    "kind": "fnOrConstructor",
                    "fnOrConstructor": {
                      "constructor": false,
                      "tsType": {
                        "repr": "void",
                        "kind": "keyword",
                        "keyword": "void"
                      },
                      "params": [
                        {
                          "kind": "identifier",
                          "name": "g",
                          "optional": false,
                          "tsType": {
                            "repr": "string",
                            "kind": "keyword",
                            "keyword": "string"
                          }
                        }
                      ],
                      "typeParams": []
                    }
                  },
                  "typeParams": []
                },
                {
                  "name": "[s]",
                  "params": [],
                  "computed": true,
                  "optional": false,
                  "tsType": {
                    "repr": "",
                    "kind": "array",
                    "array": {
                      "repr": "",
                      "kind": "union",
                      "union": [
                        {
                          "repr": "number",
                          "kind": "keyword",
                          "keyword": "number"
                        },
                        {
                          "repr": "string",
                          "kind": "keyword",
                          "keyword": "string"
                        }
                      ]
                    }
                  },
                  "typeParams": []
                }
              ],
              "callSignatures": [],
              "indexSignatures": []
            }
          },
          "kind": "const"
        }
      }
    ]
  );

  json_test!(mapped_types,
  r#"
export type MappedTypeWithNewProperties<Type> = {
  readonly [Properties in keyof Type as NewKeyType]?: Type[Properties];
};
"#;
  [
    {
      "kind": "typeAlias",
      "name": "MappedTypeWithNewProperties",
      "location": {
        "filename": "file:///test.ts",
        "line": 2,
        "col": 0,
      },
      "declarationKind": "export",
      "typeAliasDef": {
        "tsType": {
          "repr": "",
          "kind": "mapped",
          "mappedType": {
            "readonly": true,
            "typeParam": {
              "name": "Properties",
              "constraint": {
                "repr": "",
                "kind": "typeOperator",
                "typeOperator": {
                  "operator": "keyof",
                  "tsType": {
                    "repr": "Type",
                    "kind": "typeRef",
                    "typeRef": {
                      "typeParams": null,
                      "typeName": "Type"
                    }
                  }
                }
              }
            },
            "nameType": {
              "repr": "NewKeyType",
              "kind": "typeRef",
              "typeRef": {
                "typeParams": null,
                "typeName": "NewKeyType"
              }
            },
            "optional": true,
            "tsType": {
              "repr": "",
              "kind": "indexedAccess",
              "indexedAccess": {
                "readonly": false,
                "objType": {
                  "repr": "Type",
                  "kind": "typeRef",
                  "typeRef": {
                    "typeParams": null,
                    "typeName": "Type",
                  }
                },
                "indexType": {
                  "repr": "Properties",
                  "kind": "typeRef",
                  "typeRef": {
                    "typeParams": null,
                    "typeName": "Properties"
                  }
                }
              }
            }
          }
        },
        "typeParams": [
          {
            "name": "Type",
          }
        ]
      }
    }
  ]);

  json_test!(import_types,
    r#"
export function adopt<T>(p: import("./module.ts").Pet<T>) {
  console.log(`Adopting ${p.name}...`);
}
"#;
    [
      {
        "kind": "function",
        "name": "adopt",
        "location": {
          "filename": "file:///test.ts",
          "line": 2,
          "col": 0,
        },
        "declarationKind": "export",
        "functionDef": {
          "params": [
            {
              "kind": "identifier",
              "name": "p",
              "optional": false,
              "tsType": {
                "repr": "",
                "kind": "importType",
                "importType": {
                  "specifier": "./module.ts",
                  "qualifier": "Pet",
                  "typeParams": [
                    {
                      "repr": "T",
                      "kind": "typeRef",
                      "typeRef": {
                        "typeParams": null,
                        "typeName": "T"
                      }
                    }
                  ]
                }
              }
            }
          ],
          "returnType": null,
          "hasBody": true,
          "isAsync": false,
          "isGenerator": false,
          "typeParams": [
            {
              "name": "T"
            }
          ]
        }
      }
    ]
  );

  json_test!(indented_with_tabs,
      r#"
/**
 * Line 1
 *
 * Line 2
 *
 * 	Indented
 */
export namespace Tabs {
	/**
	 * Line 1
	 *
	 * Line 2
	 *
	 * 	Indented
	 */
	export interface Tabs{
		/**
		 * Line 1
		 *
		 * Line 2
		 *
		 * 	Indented
		 */
		property: string;
	}
}
    "#;
      [
        {
          "kind": "namespace",
          "name": "Tabs",
          "location": {
            "filename": "file:///test.ts",
            "line": 9,
            "col": 0
          },
          "jsDoc": {
            "doc": "Line 1\n\nLine 2\n\n\tIndented",
          },
          "declarationKind": "export",
          "namespaceDef": {
            "elements": [
              {
                "kind": "interface",
                "name": "Tabs",
                "location": {
                  "filename": "file:///test.ts",
                  "line": 17,
                  "col": 4
                },
                "jsDoc": {
                  "doc": "Line 1\n\nLine 2\n\n\tIndented",
                },
                "declarationKind": "export",
                "interfaceDef": {
                  "extends": [],
                  "methods": [],
                  "properties": [
                    {
                      "name": "property",
                      "location": {
                        "filename": "file:///test.ts",
                        "line": 25,
                        "col": 8
                      },
                      "jsDoc": {
                        "doc": "Line 1\n\nLine 2\n\n\tIndented",
                      },
                      "params": [],
                      "computed": false,
                      "optional": false,
                      "tsType": {
                        "repr": "string",
                        "kind": "keyword",
                        "keyword": "string"
                      },
                      "typeParams": []
                    }
                  ],
                  "callSignatures": [],
                  "indexSignatures": [],
                  "typeParams": []
                }
              }
            ]
          }
        }
  ]
    );

  json_test!(overloads,
  r#"
export function a(b: string): string;
export function a(b: number): number;
export function a(b: string | number): string | number {
  //
}
    "#;
  [{
    "kind": "function",
    "name": "a",
    "location": {
      "filename": "file:///test.ts",
      "line": 2,
      "col": 0
    },
    "declarationKind": "export",
    "functionDef": {
      "params": [{
        "kind": "identifier",
        "name": "b",
        "optional": false,
        "tsType": {
          "repr": "string",
          "kind": "keyword",
          "keyword": "string",
        },
      }],
      "returnType": {
        "repr": "string",
        "kind": "keyword",
        "keyword": "string",
      },
      "isAsync": false,
      "isGenerator": false,
      "typeParams": [],
    }
  }, {
    "kind": "function",
    "name": "a",
    "location": {
      "filename": "file:///test.ts",
      "line": 3,
      "col": 0
    },
    "declarationKind": "export",
    "functionDef": {
      "params": [{
        "kind": "identifier",
        "name": "b",
        "optional": false,
        "tsType": {
          "repr": "number",
          "kind": "keyword",
          "keyword": "number",
        },
      }],
      "returnType": {
        "repr": "number",
        "kind": "keyword",
        "keyword": "number",
      },
      "isAsync": false,
      "isGenerator": false,
      "typeParams": [],
    }
  }, {
    "kind": "function",
    "name": "a",
    "location": {
      "filename": "file:///test.ts",
      "line": 4,
      "col": 0
    },
    "declarationKind": "export",
    "functionDef": {
      "params": [{
        "kind": "identifier",
        "name": "b",
        "optional": false,
        "tsType": {
          "repr": "",
          "kind": "union",
          "union": [{
            "repr": "string",
            "kind": "keyword",
            "keyword": "string",
          }, {
            "repr": "number",
            "kind": "keyword",
            "keyword": "number"
          }]
        }
      }],
      "returnType": {
        "repr": "",
        "kind": "union",
        "union": [{
          "repr": "string",
          "kind": "keyword",
          "keyword": "string"
        }, {
          "repr": "number",
          "kind": "keyword",
          "keyword": "number"
        }]
      },
      "hasBody": true,
      "isAsync": false,
      "isGenerator": false,
      "typeParams": [],
    }
  }]);

  json_test!(ctor_overloads,
  r#"
export class A {
  constructor (a: string);
  constructor (a: number);
  constructor (a: string | number) {
    //
  }
}"#;
  [{
    "kind": "class",
    "name": "A",
    "location": {
      "filename": "file:///test.ts",
      "line": 2,
      "col": 0,
    },
    "declarationKind": "export",
    "classDef": {
      "isAbstract": false,
      "constructors": [{
        "accessibility": null,
        "name": "constructor",
        "params": [{
          "kind": "identifier",
          "name": "a",
          "optional": false,
          "tsType": {
            "repr": "string",
            "kind": "keyword",
            "keyword": "string",
          }
        }],
        "location": {
          "filename": "file:///test.ts",
          "line": 3,
          "col": 2,
        }
      }, {
        "accessibility": null,
        "name": "constructor",
        "params": [{
          "kind": "identifier",
          "name": "a",
          "optional": false,
          "tsType": {
            "repr": "number",
            "kind": "keyword",
            "keyword": "number",
          }
        }],
        "location": {
          "filename": "file:///test.ts",
          "line": 4,
          "col": 2,
        }
      }, {
        "accessibility": null,
        "hasBody": true,
        "name": "constructor",
        "params": [{
          "kind": "identifier",
          "name": "a",
          "optional": false,
          "tsType": {
            "repr": "",
            "kind": "union",
            "union": [{
              "repr": "string",
              "kind": "keyword",
              "keyword": "string",
            }, {
              "repr": "number",
              "kind": "keyword",
              "keyword": "number",
            }]
          }
        }],
        "location": {
          "filename": "file:///test.ts",
          "line": 5,
          "col": 2,
        }
      }],
      "properties": [],
      "indexSignatures": [],
      "methods": [],
      "extends": null,
      "implements": [],
      "typeParams": [],
      "superTypeParams": [],
    }
  }]);
}

mod printer {
  use crate::*;

  contains_test!(abstract_class,
    "export abstract class Class {}";
    "abstract class Class"
  );

  contains_test!(abstract_class_abstract_method,
    r#"
export abstract class Class {
  abstract method() {}
}
    "#;
    "abstract method()"
  );

  contains_test!(class_async_method,
    r#"
export class Class {
  async amethod(v) {}
}
    "#;
    "async amethod(v)"
  );

  contains_test!(class_constructor,
    r#"
export class Class {
  constructor(public a, readonly b) {}
}
    "#;
    "constructor(public a, readonly b)"
  );

  contains_test!(class_details,
    r#"
export class C {
  /** a doc */
  a() {}
  f: number;
}
    "#;
    "class C",
    "a()",
    "f: number"
  );

  contains_test!(class_details_all_with_private,
    r#"
export class Class {
  private pri() {}
  protected pro() {}
  public pub() {}
}
    "#,
    private;
    "private pri()",
    "protected pro()",
    "pub()"
  );

  contains_test!(class_details_only_non_private_without_private,
    r#"
export class Class {
  private pri() {}
  protected pro() {}
  public pub() {}
}
    "#;
    "protected pro()",
    "pub()"
  );

  contains_test!(class_declaration,
  "export class Class {}";
  "class Class"
  );

  contains_test!(class_extends,
    "export class Class extends Object {}";
    "class Class extends Object"
  );

  contains_test!(class_extends_implements,
    "export class Class extends Object implements Iterator, Iterable {}";
    "class Class extends Object implements Iterator, Iterable"
  );

  contains_test!(class_generic_extends_implements,
    "export class Class<A, B> extends Map<A, B> implements Iterator<A>, Iterable<B> {}";
    "class Class<A, B> extends Map<A, B> implements Iterator<A>, Iterable<B>"
  );

  contains_test!(class_getter_and_setter,
    r#"
export class Class {
  get a(): void {}
  set b(_v: void) {}
}
    "#;
    "get a(): void",
    "set b(_v: void)"
  );

  contains_test!(class_override_prop_method,
    r#"
export class C extends B {
  override a(): void {}
  override b: string;
}
    "#;
    "override a(): void",
    "override b: string"
  );

  contains_test!(class_index_signature,
    r#"
export class C {
  [key: string]: number;
}
    "#;
    "[key: string]: number"
  );

  contains_test!(class_implements,
    "export class Class implements Iterator {}";
    "class Class implements Iterator"
  );

  contains_test!(class_implements2,
    "export class Class implements Iterator, Iterable {}";
    "class Class implements Iterator, Iterable"
  );

  contains_test!(class_method,
    r#"
export class Class {
  method(v) {}
}
    "#;
    "method(v)"
  );

  contains_test!(class_property,
    r#"
export class Class {
  someproperty: bool;
  optproperty: bigint;
}
    "#;
    "someproperty: bool",
    "optproperty: bigint"
  );

  contains_test!(class_readonly_index_signature,
    r#"
export class C {
  readonly [key: string]: number;
}
    "#;
    "readonly [key: string]: number"
  );

  contains_test!(class_static_property,
    r#"
export class Class {
  static property = "";
}
    "#;
    "static property"
  );

  contains_test!(class_readonly_property,
    r#"
export class Class {
  readonly property = "";
}
    "#;
    "readonly property"
  );

  contains_test!(class_private_property,
    r#"
export class Class {
  private property = "";
}
    "#,
    private;
    "private property"
  );

  contains_test!(class_decorators,
    r#"
@sealed
export class A {
  #x = "x";

  @format("Hello, %s")
  greeting: string;

  @configurable(false)
  get x() {
    return this.#x;
  }

  @enumerable(false)
  greet() {
    return "hello";
  }
}
    "#;
    "@sealed",
    "@format(\"Hello, %s\")",
    "@configurable(false)",
    "@enumerable(false)"
  );

  contains_test!(class_method_overloads,
    r#"export class A {
      a(b: string): string;
      a(b: number): number;
      a(b: string | number): string | number {}
    }"#;
    "a(b: string): string",
    "a(b: number): number";
    "a(b: string | number): string | number"
  );

  contains_test!(const_declaration,
  "export const Const = 0;";
    "const Const"
  );

  contains_test!(enum_declaration,
  "export enum Enum {}";
    "enum Enum"
  );

  contains_test!(enum_member,
    r#"
export enum Enum {
  First,
  /** There is a JsDoc */
  Second,
}
    "#;
    "enum Enum",
    "First",
    "Second",
    "There is a JsDoc"
  );

  contains_test!(exports_all_with_private,
    r#"
export function a() {}
function b() {}
export class C {}
class D {}
export interface E {}
interface F {}
export namespace G {}
namespace H {}
    "#,
    private;
    "function a()",
    "class C",
    "interface E",
    "namespace G",
    "function b()",
    "class D",
    "interface F",
    "namespace H"
  );

  contains_test!(exports_declared_earlier,
    r#"
const hello = "world";
function say(words: string): void { }
function foo(): void { }
export { hello, say, foo as bar };
    "#;
    "const hello",
    "function say(words: string): void",
    "function bar(): void"
  );

  contains_test!(function_async,
    "export async function a() {}";
    "async function a()"
  );

  contains_test!(function_array_deconstruction,
    "export function f([a, b, ...c]) {}";
    "function f([a, b, ...c])"
  );

  contains_test!(function_async_generator,
    "export async function* ag() {}";
    "async function* ag()"
  );

  contains_test!(function_declaration,
  "export function fun() {}";
    "function fun()"
  );

  contains_test!(function_generator,
    "export function* g() {}";
    "function* g()"
  );

  contains_test!(function_generic,
    "export function add<T>(a: T, b: T) { return a + b; }";
    "function add<T>(a: T, b: T)"
  );

  contains_test!(function_object_deconstruction,
    "export function f({ a, b, ...c }) {}";
    "function f({a, b, ...c})"
  );

  /* TODO(SyrupThinker) NYI
  contains_test!(function_type_predicate,
    r#"
  export function isFish(pet: Fish | Bird): pet is Fish {
      return (pet as Fish).swim !== undefined;
  }
    "#;
    "pet is Fish"
  );
  */

  contains_test!(function_overloads,
    r#"
export function a(b: string): string;
export function a(b: number): number;
export function a(b: string | number): string | number {}
"#;
    "function a(b: string): string",
    "function a(b: number): number";
    "function a(b: string | number): string | number"
  );

  contains_test!(generic_instantiated_with_tuple_type,
    r#"
export interface Generic<T> {}
export function f(): Generic<[string, number]> { return {}; }
    "#;
    "function f(): Generic<[string, number]>"
  );

  contains_test!(type_literal_declaration,
    "export type T = {}";
    "{ }"
  );

  contains_test!(type_literal_index_signature,
    "export type T = { [key: string]: number; }";
    "[key: string]: number"
  );

  contains_test!(type_literal_readonly_index_signature,
    "export type T = { readonly [key: string]: number; }";
    "readonly [key: string]: number"
  );

  contains_test!(type_alias_infer_type,
    "export type Flatten<T> = T extends Array<infer U> ? U : T;";
    "T extends Array<infer U> ? U : T"
  );

  contains_test!(type_literal_mapped_type,
    "export type T<Type> = { readonly [P in keyof Type as NewType]: Type[P]; }";
    "readonly [P in keyof Type as NewType]: Type[P]"
  );

  contains_test!(type_import_type,
    "export function adopt<T>(p: import(\"./module.ts\").Pet<T>) { }";
    "import(\"./module.ts\").Pet<T>"
  );

  contains_test!(interface_declaration,
  "export interface Interface {}";
    "interface Interface"
  );

  contains_test!(interface_extends,
    "export interface Interface extends Iterator {}";
    "interface Interface extends Iterator"
  );

  contains_test!(interface_extends2,
    "export interface Interface extends Iterator, Iterable {}";
    "interface Interface extends Iterator, Iterable"
  );

  contains_test!(interface_generic,
    "export interface Interface<T> {}";
    "interface Interface<T>"
  );

  contains_test!(interface_generic_extends,
    "export interface Interface<V> extends Iterable<V> {}";
    "interface Interface<V> extends Iterable<V>"
  );

  contains_test!(interface_index_signature,
    r#"
export interface Interface {
  [index: number]: Interface;
}
    "#;
    "[index: number]: Interface"
  );

  contains_test!(interface_method,
    r#"
export interface I {
  m(a, b);
  mo?(c);
  [mc](d);
}
    "#;
    "m(a, b)",
    "mo?(c)",
    "[mc](d)"
  );

  contains_test!(interface_property,
    r#"
export interface I {
  p: string;
  po?: number;
  readonly pro: string;
  [pc]: string;
}
    "#;
    "p: string",
    "po?: number",
    "readonly pro: string",
    "[pc]: string"
  );

  contains_test!(interface_string_literal_property,
    r#"
export interface I {
  "p": string;
  "po"?: number;
}
    "#;
    "p: string",
    "po?: number"
  );

  contains_test!(interface_number_literal_property,
    r#"
export interface I {
  1: string;
  2?: number;
}
    "#;
    "1: string",
    "2?: number"
  );

  contains_test!(interface_readonly_index_signature,
    r#"
export interface Interface {
  readonly [index: number]: Interface;
}
    "#;
    "readonly [index: number]: Interface"
  );

  contains_test!(interface_construct,
    r#"
export interface I {
  new(name: string);
}
    "#;
    "new(name: string)"
  );

  contains_test!(jsdoc,
    r#"
/**
 * A is a class
 *
 * Nothing more
 */
export class A {}
/**
 * B is an interface
 *
 * Should be
 */
export interface B {}
/**
 * C is a function
 *
 * Summarised
 */
export function C() {}
    "#;
    "A is a class",
    "B is an interface",
    "C is a function",
    "Nothing more",
    "Should be",
    "Summarised"
  );

  contains_test!(jsdoc_tags,
    r#"
/**
 * a is a function
 *
 * @param {string} b some doc
 * @param [c=1] additional doc
 * @param [d] more doc
 * @returns {string} returning doc
 */
export function a(b, c, d) {}
    "#;
    "a is a function",
    "@param {string} b",
    "some doc",
    "@param [1] c",
    "additional doc",
    "@param [?] d",
    "more doc",
    "@return {string}",
    "returning doc"
  );

  contains_test!(namespace_declaration,
  "export namespace Namespace {}";
    "namespace Namespace"
  );

  contains_test!(namespace_details,
    r#"
export namespace Namespace {
  /**
   * Doc comment 1
   *
   * Details 1
   */
  export function a() {}
  /**
   * Doc comment 2
   *
   * Details 2
   */
  export class B {}
}
    "#;
    "namespace Namespace",
    "function a()",
    "class B",
    "Doc comment 1",
    "Doc comment 2",
    "Details 1",
    "Details 2"
  );

  contains_test!(namespace_fn_overloads,
    r#"
export namespace Namespace {
  export function a(b: string): string;
  export function a(b: number): number;
  export function a(b: number | string): number | string {}
}
"#;
    "function a(b: string): string",
    "function a(b: number): number";
    "function a(b: number | string): number | string"
  );

  contains_test!(type_alias,
  "export type A = number";
  "type A = number"
  );

  contains_test!(type_generic_alias,
  "export type A<T> = T";
  "type A<T> = T"
  );

  contains_test!(infer_ts_types,
    r#"
    export let s = "hello";
    export let n = 123;
    export let b = false;
    export let bi = 100n;
    export let re = /hello/;
    export let tpl = `foobar`;
    "#;
    "let s: string",
    "let n: number",
    "let b: boolean",
    "let bi: bigint",
    "let re: RegExp",
    "let tpl: string"
  );

  contains_test!(infer_simple_ts_types,
    r#"
export const s = "hello";
export const n = 123;
export const b = false;
export const bi = 100n;
export const re = /hello/;
export const tpl = `foobar`;
export const d = new Date();
export const s2 = String("foo");
export const n2 = Number(100);
export const bi2 = BigInt(123);
export const sym = Symbol("hello");
export const m = new Map<string, string>();
export const fn1 = (a: string): void => {};
export const fn2 = function (a:string): void {};
export const s3 = "VGhpcyBpcyBhIHJlYWxseSBsb25nIHN0cmluZyB0byB0cnkgdG8gZmluZCBvdXQgaWYgd2UgY2FuIHNraXAgaW5mZXJyaW5nIGEgcmVhbGx5IGxhcmdlIHZhbHVlIGZvciBhIGNvbnN0IHN0cmluZyB2YXJpYWJsZSB3aXRob3V0IGEgdHlwZSBhbm5vdGF0aW9uLg==";
    "#;
    "const s: \"hello\"",
    "const n: 123",
    "const b: false",
    "const bi: 100",
    "const re: RegExp",
    "const tpl: `foobar`",
    "const d: Date",
    "const s2: string",
    "const n2: number",
    "const bi2: bigint",
    "const sym: symbol",
    "const m: Map<string, string>",
    "const fn1: (a: string) => void",
    "const fn2: (a: string) => void",
    "const s3: string"
  );

  contains_test!(infer_simple_ts_arr_types,
    r#"
      export const a = [1];
      export const b = [true];
      export const c = ["c"];
      export const d = [2, "d"];
      export const e = [1, 2, 3];
      export const f = ["a", 1, e];
      export const g = ["a", 1, true] as const;
    "#;
    "const a: number[]",
    "const b: boolean[]",
    "const c: string[]",
    "const d: (number | string)[]",
    "const e: number[]",
    "const f: any[]",
    "const g: (\"a\" | 1 | true)[]"
  );

  contains_test!(
    ts_template_with_args,
    r#"
export const tpl: `test${number}` = `test1`;
    "#;
    "const tpl: `test${number}`"
  );

  contains_test!(
    ts_user_defined_type_guards,
    r#"
export function f1(val1: A | B): val1 is A {}
export function f2(val2: any): asserts val2 is string {}
export function f3(val3: any): asserts val3 {}
export function assertIsDefined<T>(val4: T): asserts val4 is NonNullable<T> {
  if (val === undefined || val === null) {
    throw new AssertionError(
      `Expected 'val' to be defined, but received ${val}`
    );
  }
}
export class C {
  isSomething(): this is Something {
    return this instanceof Something;
  }
}
    "#;
    "val1 is A",
    "asserts val2 is string",
    "asserts val3",
    "asserts val4 is NonNullable<T>",
    "this is Something"
  );

  contains_test!(import_equals,
    "declare module Test {
  export interface Options {
  }
}

import Options = Test.Options;

export { Options };";
    "interface Options"
  );
}

fn create_analyzer() -> CapturingModuleAnalyzer {
  let source_parser = DefaultModuleParser::new_for_analysis();
  CapturingModuleAnalyzer::new(Some(Box::new(source_parser)), None)
}
