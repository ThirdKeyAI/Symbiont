use dsl::{extract_metadata, find_errors, parse_dsl, print_ast};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Symbiont DSL Parser");
    println!("==================");

    // Start with a very simple test case
    let simple_test = r#"metadata {
    version: "1.0"
}"#;

    println!("Testing simple metadata block...\n");

    // Parse the simple DSL
    match parse_dsl(simple_test) {
        Ok(tree) => {
            println!("✅ Simple parsing successful!");
            println!("\n📊 Simple AST:");
            println!("===============");
            print_ast(tree.root_node(), simple_test, 0);

            // Check for parsing errors
            let root_node = tree.root_node();
            if root_node.has_error() {
                println!("\n⚠️  Warning: Parse tree contains errors");
                for diag in find_errors(root_node, simple_test, 0) {
                    println!("  {}", diag);
                }
            } else {
                println!("\n✅ No parsing errors detected!");

                // Now try the full sample
                let sample_dsl = r#"
// Sample Symbiont DSL program
metadata {
    version: "1.0",
    author: "AI Assistant",
    description: "Sample DSL demonstration"
}

agent DataProcessor {
    capabilities: [read, write, transform]
    
    policy ProcessingPolicy {
        allow: read(data_source)
        require: validate(input)
        deny: delete(critical_data)
    }
    
    function process_data(input: String) -> Result<String> {
        let validated = validate(input);
        if validated {
            return transform(input);
        } else {
            return error("Invalid input");
        }
    }
}

type DataSource = {
    url: String,
    format: String,
    credentials: Option<String>
}

agent APIGateway {
    capabilities: [route, authenticate, log]
    
    policy SecurityPolicy {
        require: authenticate(request)
        allow: route(authenticated_request)
        audit: log(all_requests)
    }
}
"#;

                println!("\n\nTesting full DSL sample...\n");

                match parse_dsl(sample_dsl) {
                    Ok(full_tree) => {
                        println!("✅ Full parsing successful!");
                        println!("\n📊 Full Abstract Syntax Tree:");
                        println!("=============================");
                        print_ast(full_tree.root_node(), sample_dsl, 0);

                        println!("\n📋 Extracted Metadata:");
                        println!("======================");
                        let metadata = extract_metadata(&full_tree, sample_dsl);
                        for (key, value) in &metadata {
                            println!("  {}: {}", key, value);
                        }

                        if metadata.is_empty() {
                            println!("  No metadata found in the DSL code.");
                        }

                        if full_tree.root_node().has_error() {
                            println!("\n⚠️  Warning: Full parse tree contains errors");
                            for diag in find_errors(full_tree.root_node(), sample_dsl, 0) {
                                println!("  {}", diag);
                            }
                        } else {
                            println!("\n✅ No parsing errors in full sample!");
                        }
                    }
                    Err(e) => {
                        println!("❌ Full parsing failed: {}", e);
                    }
                }
            }
        }
        Err(e) => {
            println!("❌ Simple parsing failed: {}", e);
            return Err(e);
        }
    }

    println!("\n🎉 DSL Parser demonstration completed!");
    Ok(())
}
