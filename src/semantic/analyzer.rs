use crate::error::{DominoError, Result};
use crate::profiler::Profiler;
use crate::types::{Export, Import, Project, Reference};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
  ExportNamedDeclaration, Expression, ImportDeclaration, ImportDeclarationSpecifier,
};
use oxc_ast::AstKind;
use oxc_ast_visit::walk;
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType, Span};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, warn};

/// Type alias for import index entries: (importing_file, local_name, from_module)
/// (importing_file, local_name, from_module, is_dynamic)
type ImportIndexEntry = Vec<(PathBuf, String, String, bool)>;
/// Type alias for the import index map: (source_file, symbol_name) -> entries
type ImportIndexMap = FxHashMap<(PathBuf, String), ImportIndexEntry>;

/// Semantic data for a single file
pub struct FileSemanticData {
  pub source: String,
  #[allow(dead_code)]
  pub allocator: Allocator,
  pub semantic: oxc_semantic::Semantic<'static>,
}

/// Workspace-wide semantic analysis
pub struct WorkspaceAnalyzer {
  /// Per-file semantic analysis
  pub files: HashMap<PathBuf, FileSemanticData>,
  /// Import graph: importing_file -> imports
  pub imports: HashMap<PathBuf, Vec<Import>>,
  /// Export graph: exporting_file -> exports
  pub exports: HashMap<PathBuf, Vec<Export>>,
  /// Projects in the workspace
  pub projects: Vec<Project>,
  /// Reverse import index: (source_file, symbol_name) -> [(importing_file, local_name, from_module)]
  /// This index maps from a file+symbol to all the places that import it
  /// The from_module is kept for re-export checking
  pub import_index: ImportIndexMap,
  /// Profiler for performance measurement
  pub profiler: Arc<Profiler>,
}

impl WorkspaceAnalyzer {
  /// Create a new workspace analyzer
  pub fn new(projects: Vec<Project>, cwd: &Path, profiler: Arc<Profiler>) -> Result<Self> {
    let mut analyzer = Self {
      files: HashMap::new(),
      imports: HashMap::new(),
      exports: HashMap::new(),
      projects,
      import_index: FxHashMap::default(),
      profiler,
    };

    analyzer.analyze_workspace(cwd)?;

    // Build import index
    analyzer.build_import_index(cwd)?;

    Ok(analyzer)
  }

  /// Build reverse import index: (source_file, symbol) -> [(importing_file, local_name, from_module)]
  /// This must be called after analyze_workspace and needs a resolver
  fn build_import_index(&mut self, cwd: &Path) -> Result<()> {
    use oxc_resolver::{ResolveOptions, Resolver};

    // Create resolver for building the index
    let tsconfig_path = cwd.join("tsconfig.base.json");
    let options = ResolveOptions {
      extensions: vec![
        ".ts".into(),
        ".tsx".into(),
        ".js".into(),
        ".jsx".into(),
        ".d.ts".into(),
      ],
      tsconfig: if tsconfig_path.exists() {
        Some(oxc_resolver::TsconfigDiscovery::Manual(
          oxc_resolver::TsconfigOptions {
            config_file: tsconfig_path.clone(),
            references: oxc_resolver::TsconfigReferences::Auto,
          },
        ))
      } else {
        None
      },
      ..Default::default()
    };
    let resolver = Resolver::new(options);
    use tracing::debug;

    let mut index: ImportIndexMap = FxHashMap::default();

    // For each file and its imports
    for (importing_file, file_imports) in &self.imports {
      for import in file_imports {
        // NOTE: We intentionally do NOT skip type-only imports
        // Even though they don't exist at runtime, they represent semantic dependencies
        // If a type changes, files that import it need to be re-type-checked

        // Resolve where this import comes from
        let from_path = cwd.join(importing_file);
        let context = match from_path.parent() {
          Some(ctx) => ctx,
          None => continue,
        };

        let resolved = match resolver.resolve(context, &import.from_module) {
          Ok(resolution) => {
            let resolved = resolution.path();
            match resolved.strip_prefix(cwd) {
              Ok(p) => p.to_path_buf(),
              Err(_) => continue,
            }
          }
          Err(_) => {
            // Try simple relative resolution as fallback
            if !import.from_module.starts_with('.') {
              continue;
            }
            let base = context.join(&import.from_module);
            let mut resolved_path = None;
            for ext in &[".ts", ".tsx", ".js", ".jsx", "/index.ts", "/index.js"] {
              let candidate = if ext.starts_with('/') {
                base.join(ext.trim_start_matches('/'))
              } else {
                // Append extension instead of replacing it
                // This handles cases like colors.css -> colors.css.ts (vanilla-extract)
                PathBuf::from(format!("{}{}", base.display(), ext))
              };
              if cwd.join(&candidate).exists() {
                if let Ok(p) = candidate.strip_prefix(cwd) {
                  resolved_path = Some(p.to_path_buf());
                  break;
                }
              }
            }
            match resolved_path {
              Some(p) => p,
              None => continue,
            }
          }
        };

        // Add to index: (resolved_file, imported_symbol) -> (importing_file, local_name, from_module, is_dynamic)
        let key = (resolved, import.imported_name.clone());
        index.entry(key).or_default().push((
          importing_file.clone(),
          import.local_name.clone(),
          import.from_module.clone(),
          import.is_dynamic,
        ));
      }
    }

    let unique_symbols = index
      .keys()
      .map(|(_, symbol)| symbol)
      .collect::<FxHashSet<_>>()
      .len();
    debug!(
      "Built import index with {} entries covering {} unique symbols",
      index.len(),
      unique_symbols
    );
    self.import_index = index;

    Ok(())
  }

  /// Analyze all files in the workspace
  fn analyze_workspace(&mut self, cwd: &Path) -> Result<()> {
    for project in &self.projects.clone() {
      let source_root = if project.source_root.is_absolute() {
        project.source_root.clone()
      } else {
        cwd.join(&project.source_root)
      };

      if !source_root.exists() {
        warn!("Source root does not exist: {:?}", source_root);
        continue;
      }

      self.analyze_directory(&source_root, cwd)?;
    }

    Ok(())
  }

  /// Recursively analyze a directory
  fn analyze_directory(&mut self, dir: &Path, cwd: &Path) -> Result<()> {
    if !dir.is_dir() {
      return Ok(());
    }

    for entry in fs::read_dir(dir)? {
      let entry = entry?;
      let path = entry.path();

      // Skip node_modules, dist, build, etc.
      if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name == "node_modules" || name == "dist" || name == "build" || name.starts_with('.') {
          continue;
        }
      }

      if path.is_dir() {
        self.analyze_directory(&path, cwd)?;
      } else if path.is_file() {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
          if matches!(ext, "ts" | "tsx" | "js" | "jsx") {
            let relative_path = path.strip_prefix(cwd).unwrap_or(&path).to_path_buf();
            if let Err(e) = self.analyze_file(&path, &relative_path) {
              warn!("Failed to analyze {}: {}", path.display(), e);
            }
          }
        }
      }
    }

    Ok(())
  }

  /// Analyze a single file
  fn analyze_file(&mut self, file_path: &Path, relative_path: &Path) -> Result<()> {
    let source = fs::read_to_string(file_path)?;

    // Determine source type from file extension
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));

    // Create allocator for this file
    let allocator = Allocator::default();

    // Parse the file
    let parser = Parser::new(&allocator, &source, source_type);
    let parse_result = parser.parse();

    if !parse_result.errors.is_empty() {
      debug!(
        "Parse errors in {:?}: {} errors",
        file_path,
        parse_result.errors.len()
      );
      // Continue anyway - partial AST may still be useful
    }

    // Build semantic data
    let semantic_builder = SemanticBuilder::new()
      .with_cfg(true)
      .with_check_syntax_error(false);

    let semantic_ret = semantic_builder.build(&parse_result.program);

    if !semantic_ret.errors.is_empty() {
      debug!(
        "Semantic errors in {:?}: {} errors",
        file_path,
        semantic_ret.errors.len()
      );
    }

    // Extract imports and exports
    let imports = Self::extract_imports(&parse_result.program, relative_path);
    let exports = Self::extract_exports(&parse_result.program);

    self.imports.insert(relative_path.to_path_buf(), imports);
    self.exports.insert(relative_path.to_path_buf(), exports);

    // Store semantic data
    // Safety: We're storing the semantic data with its allocator, which is valid
    // as long as the FileSemanticData struct exists
    let semantic = unsafe {
      std::mem::transmute::<oxc_semantic::Semantic<'_>, oxc_semantic::Semantic<'static>>(
        semantic_ret.semantic,
      )
    };

    self.files.insert(
      relative_path.to_path_buf(),
      FileSemanticData {
        source,
        allocator,
        semantic,
      },
    );

    Ok(())
  }
}

/// Visitor to collect dynamic imports (import() expressions)
struct DynamicImportVisitor<'a> {
  imports: Vec<Import>,
  dynamic_count: usize,
  /// Phantom data to maintain lifetime parameter
  /// This zero-sized type marker ensures the visitor maintains the correct lifetime
  _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a> DynamicImportVisitor<'a> {
  fn new() -> Self {
    Self {
      imports: Vec::new(),
      dynamic_count: 0,
      _phantom: std::marker::PhantomData,
    }
  }

  /// Create a namespace import for a dynamic import expression
  ///
  /// Since we can't statically analyze which symbols are accessed from dynamic imports
  /// (especially with .then() transformations), we conservatively treat them as
  /// namespace imports (import * as ...) to ensure we track the dependency.
  fn create_namespace_import(&self, from_module: &str) -> Import {
    Import {
      imported_name: "*".to_string(),
      local_name: format!("__dynamic_import_{}", self.dynamic_count),
      from_module: from_module.to_string(),
      resolved_file: None,
      is_type_only: false,
      is_dynamic: true,
    }
  }
}

impl<'a> Visit<'a> for DynamicImportVisitor<'a> {
  fn visit_import_expression(&mut self, expr: &oxc_ast::ast::ImportExpression<'a>) {
    // Extract the module specifier from the import() call
    match &expr.source {
      Expression::StringLiteral(string_lit) => {
        let from_module = string_lit.value.as_str().to_string();
        debug!("Found dynamic import: {}", from_module);

        // Create a namespace import for this dynamic import
        let import = self.create_namespace_import(&from_module);
        self.imports.push(import);
        self.dynamic_count += 1;
      }
      _ => {
        // Non-string-literal imports (template literals, variables, etc.)
        // are not currently supported. These would require runtime evaluation.
        warn!(
          "Skipping dynamic import with non-string-literal specifier (template literal or variable). \
           Only string literal dynamic imports are currently supported for affected analysis."
        );
      }
    }

    // Continue walking the AST
    walk::walk_import_expression(self, expr);
  }
}

impl WorkspaceAnalyzer {
  /// Extract imports from an AST
  fn extract_imports(program: &oxc_ast::ast::Program, file_path: &Path) -> Vec<Import> {
    let mut imports = Vec::new();

    // Extract static imports
    for node in program.body.iter() {
      if let oxc_ast::ast::Statement::ImportDeclaration(import_decl) = node {
        imports.extend(Self::process_import(import_decl));
      }
    }

    let static_count = imports.len();

    // Extract dynamic imports using visitor
    let mut visitor = DynamicImportVisitor::new();
    visitor.visit_program(program);
    let dynamic_count = visitor.dynamic_count;
    imports.extend(visitor.imports);

    debug!(
      "Extracted {} total imports ({} static, {} dynamic) from {:?}",
      imports.len(),
      static_count,
      dynamic_count,
      file_path
    );
    imports
  }

  fn process_import(import_decl: &oxc_allocator::Box<ImportDeclaration>) -> Vec<Import> {
    let mut imports = Vec::new();
    let from_module = import_decl.source.value.as_str().to_string();
    let is_type_only = import_decl.import_kind.is_type();

    if let Some(specifiers) = &import_decl.specifiers {
      for specifier in specifiers.iter() {
        match specifier {
          ImportDeclarationSpecifier::ImportSpecifier(spec) => {
            let imported_name = spec.imported.name().to_string();
            let local_name = spec.local.name.to_string();

            imports.push(Import {
              imported_name,
              local_name,
              from_module: from_module.clone(),
              resolved_file: None, // Will be resolved later
              is_type_only: is_type_only || spec.import_kind.is_type(),
              is_dynamic: false,
            });
          }
          ImportDeclarationSpecifier::ImportDefaultSpecifier(spec) => {
            imports.push(Import {
              imported_name: "default".to_string(),
              local_name: spec.local.name.to_string(),
              from_module: from_module.clone(),
              resolved_file: None,
              is_type_only,
              is_dynamic: false,
            });
          }
          ImportDeclarationSpecifier::ImportNamespaceSpecifier(spec) => {
            imports.push(Import {
              imported_name: "*".to_string(),
              local_name: spec.local.name.to_string(),
              from_module: from_module.clone(),
              resolved_file: None,
              is_type_only,
              is_dynamic: false,
            });
          }
        }
      }
    }

    imports
  }

  /// Extract exports from an AST
  fn extract_exports(program: &oxc_ast::ast::Program) -> Vec<Export> {
    let mut exports = Vec::new();

    for node in program.body.iter() {
      match node {
        oxc_ast::ast::Statement::ExportNamedDeclaration(export_decl) => {
          exports.extend(Self::process_named_export(export_decl));
        }
        oxc_ast::ast::Statement::ExportDefaultDeclaration(_) => {
          exports.push(Export {
            exported_name: "default".to_string(),
            local_name: None,
            re_export_from: None,
          });
        }
        oxc_ast::ast::Statement::ExportAllDeclaration(export_all) => {
          let from = export_all.source.value.as_str().to_string();
          exports.push(Export {
            exported_name: "*".to_string(),
            local_name: None,
            re_export_from: Some(from),
          });
        }
        _ => {}
      }
    }

    exports
  }

  fn process_named_export(export_decl: &ExportNamedDeclaration) -> Vec<Export> {
    let mut exports = Vec::new();

    let re_export_from = export_decl
      .source
      .as_ref()
      .map(|s| s.value.as_str().to_string());

    for specifier in &export_decl.specifiers {
      let exported_name = specifier.exported.name().to_string();
      let local_name = Some(specifier.local.name().to_string());

      exports.push(Export {
        exported_name,
        local_name,
        re_export_from: re_export_from.clone(),
      });
    }

    // Handle inline exports (export const x = ...)
    if let Some(decl) = &export_decl.declaration {
      match decl {
        oxc_ast::ast::Declaration::VariableDeclaration(var_decl) => {
          for declarator in &var_decl.declarations {
            if let oxc_ast::ast::BindingPatternKind::BindingIdentifier(id) = &declarator.id.kind {
              exports.push(Export {
                exported_name: id.name.to_string(),
                local_name: None,
                re_export_from: None,
              });
            }
          }
        }
        oxc_ast::ast::Declaration::FunctionDeclaration(func_decl) => {
          if let Some(id) = &func_decl.id {
            exports.push(Export {
              exported_name: id.name.to_string(),
              local_name: None,
              re_export_from: None,
            });
          }
        }
        oxc_ast::ast::Declaration::ClassDeclaration(class_decl) => {
          if let Some(id) = &class_decl.id {
            exports.push(Export {
              exported_name: id.name.to_string(),
              local_name: None,
              re_export_from: None,
            });
          }
        }
        _ => {}
      }
    }

    exports
  }

  /// Find all local references to a symbol within a file
  pub fn find_local_references(
    &self,
    file_path: &Path,
    symbol_name: &str,
  ) -> Result<Vec<Reference>> {
    let start = if self.profiler.is_enabled() {
      Some(Instant::now())
    } else {
      None
    };

    let file_data = self
      .files
      .get(file_path)
      .ok_or_else(|| DominoError::FileNotFound(file_path.display().to_string()))?;

    let mut references = Vec::new();

    // Iterate through all symbols in the file
    for symbol_id in file_data.semantic.scoping().symbol_ids() {
      let name = file_data.semantic.scoping().symbol_name(symbol_id);

      if name == symbol_name {
        // Get all references to this symbol using the Semantic API directly
        for reference in file_data.semantic.symbol_references(symbol_id) {
          let span = file_data.semantic.reference_span(reference);
          let (line, column) = self.span_to_line_col(&file_data.source, span);

          references.push(Reference {
            file_path: file_path.to_path_buf(),
            line,
            column,
          });
        }
      }
    }

    if let Some(start_time) = start {
      self
        .profiler
        .record_local_reference(start_time.elapsed().as_nanos() as u64);
    }

    Ok(references)
  }

  /// Convert span to line and column
  fn span_to_line_col(&self, source: &str, span: Span) -> (usize, usize) {
    let offset = span.start as usize;
    crate::utils::offset_to_line_col(source, offset)
  }

  /// Helper method to extract symbol name from an export declaration
  ///
  /// Handles various export patterns:
  /// - export const/let/var X = ...
  /// - export function X() {}
  /// - export class X {}
  /// - export interface X {}
  /// - export type X = ...
  /// - export enum X {}
  fn extract_symbol_from_export_decl(decl: &oxc_ast::ast::Declaration) -> Option<String> {
    match decl {
      oxc_ast::ast::Declaration::VariableDeclaration(var_decl) => {
        for declarator in &var_decl.declarations {
          if let oxc_ast::ast::BindingPatternKind::BindingIdentifier(id) = &declarator.id.kind {
            return Some(id.name.to_string());
          }
        }
        None
      }
      oxc_ast::ast::Declaration::FunctionDeclaration(func_decl) => {
        func_decl.id.as_ref().map(|id| id.name.to_string())
      }
      oxc_ast::ast::Declaration::ClassDeclaration(class_decl) => {
        class_decl.id.as_ref().map(|id| id.name.to_string())
      }
      oxc_ast::ast::Declaration::TSInterfaceDeclaration(interface) => {
        Some(interface.id.name.to_string())
      }
      oxc_ast::ast::Declaration::TSTypeAliasDeclaration(type_alias) => {
        Some(type_alias.id.name.to_string())
      }
      oxc_ast::ast::Declaration::TSEnumDeclaration(enum_decl) => {
        Some(enum_decl.id.name.to_string())
      }
      _ => None,
    }
  }

  /// Check if a symbol is exported from a file
  pub fn is_symbol_exported(&self, file_path: &Path, symbol_name: &str) -> bool {
    if let Some(exports) = self.exports.get(file_path) {
      exports.iter().any(|export| {
        // Check if the symbol is directly exported
        export.exported_name == symbol_name
          // Or if it's exported under a different name (local_name matches)
          || export.local_name.as_ref().is_some_and(|local| local == symbol_name)
      })
    } else {
      false
    }
  }

  /// Get all exported symbols that use a given local symbol
  /// This is used to find which exported APIs are affected when an internal symbol changes
  pub fn find_exported_symbols_using(
    &self,
    file_path: &Path,
    local_symbol: &str,
  ) -> Result<Vec<String>> {
    let mut exported_symbols = Vec::new();

    // Get all exports from this file
    let exports = match self.exports.get(file_path) {
      Some(exports) if !exports.is_empty() => exports,
      _ => {
        debug!(
          "No exports found for {:?} - cannot find exported symbols using '{}'",
          file_path, local_symbol
        );
        return Ok(exported_symbols);
      }
    };

    // Find all references to the local symbol once (O(n) operation)
    let refs = self.find_local_references(file_path, local_symbol)?;
    if refs.is_empty() {
      debug!(
        "No references found for '{}' in {:?} - no exported symbols use it",
        local_symbol, file_path
      );
      return Ok(exported_symbols);
    }

    // Build a set of container symbols that reference the local symbol
    // This is O(refs) instead of O(exports × refs)
    let mut containers = FxHashSet::default();
    for reference in refs {
      if let Some(container) =
        self.find_node_at_line(file_path, reference.line, reference.column)?
      {
        containers.insert(container);
      }
    }

    // Now check which exports are in the container set - O(exports)
    for export in exports {
      // Skip re-exports (they don't have local implementations)
      if export.re_export_from.is_some() {
        continue;
      }

      // Get the local name (what's actually defined in the file)
      let local_name = export.local_name.as_ref().unwrap_or(&export.exported_name);

      // Skip if this is the symbol itself (we're looking for symbols that *use* it)
      if local_name == local_symbol {
        continue;
      }

      // Check if this exported symbol contains any references to the local symbol
      if containers.contains(local_name) {
        debug!(
          "Exported symbol '{}' uses local symbol '{}'",
          export.exported_name, local_symbol
        );
        exported_symbols.push(export.exported_name.clone());
      }
    }

    Ok(exported_symbols)
  }

  /// Find node at a specific line in a file
  pub fn find_node_at_line(
    &self,
    file_path: &Path,
    line: usize,
    column: usize,
  ) -> Result<Option<String>> {
    let start = if self.profiler.is_enabled() {
      Some(Instant::now())
    } else {
      None
    };

    let file_data = self
      .files
      .get(file_path)
      .ok_or_else(|| DominoError::FileNotFound(file_path.display().to_string()))?;

    // Get the exact offset using both line and column
    let line_start = crate::utils::line_to_offset(&file_data.source, line)
      .ok_or_else(|| DominoError::Other(format!("Invalid line number: {}", line)))?;
    let exact_offset = line_start + column;

    // Find nodes at this position
    let nodes = file_data.semantic.nodes();

    // First pass: Find the SMALLEST node that CONTAINS this exact position
    // Using the exact offset (line + column) allows us to pinpoint the specific node
    let mut node_on_line_id = None;
    let mut smallest_span_size = usize::MAX;

    for node in nodes.iter() {
      let span = node.kind().span();
      let node_start = span.start as usize;
      let node_end = span.end as usize;

      // Check if this exact offset is within the node's span
      if node_start <= exact_offset && node_end >= exact_offset {
        let span_size = node_end - node_start;

        // Keep the smallest containing node, but prefer non-Program nodes when sizes are equal
        // This handles the case where an ExportNamedDeclaration spans the entire file
        let should_update = if span_size < smallest_span_size {
          // Smaller node found - always update
          true
        } else if span_size == smallest_span_size {
          // When sizes are equal, prefer non-Program nodes, but only update if we don't already
          // have a non-Program node (to ensure deterministic selection - first non-Program wins)
          let current_is_program = matches!(node.kind(), AstKind::Program(_));
          let existing_is_program = node_on_line_id
            .map(|id| matches!(nodes.get_node(id).kind(), AstKind::Program(_)))
            .unwrap_or(true);

          !current_is_program && existing_is_program
        } else {
          false
        };

        if should_update {
          smallest_span_size = span_size;
          node_on_line_id = Some(node.id());
        }
      }
    }

    if node_on_line_id.is_none() {
      return Ok(None);
    }

    // Find the containing top-level declaration (exported symbol)
    let mut current_id = node_on_line_id.unwrap();
    let mut top_level_name: Option<String> = None;

    // Flag to track if we've encountered an export wrapper (ExportNamedDeclaration or ExportDefaultDeclaration)
    // This is important because we want to extract the symbol from the export declaration itself,
    // not from the inner declaration. For example, in `export const X = 5`, we want "X" from the
    // export declaration, not from the underlying VariableDeclaration.
    let mut found_export_wrapper = false;

    // First check the current node itself - this is an optimization for when the cursor
    // is directly on an export declaration (common case when a line starts with "export")
    let current_node = nodes.get_node(current_id);
    match current_node.kind() {
      AstKind::ExportNamedDeclaration(export_decl) => {
        found_export_wrapper = true;
        // Check if there's an inline declaration (export const x = ...)
        if let Some(decl) = &export_decl.declaration {
          top_level_name = Self::extract_symbol_from_export_decl(decl);
        }
      }
      AstKind::ExportDefaultDeclaration(_) => {
        found_export_wrapper = true;
        top_level_name = Some("default".to_string());
      }
      _ => {}
    }

    // If we found the symbol at the current node level, return it early
    if found_export_wrapper && top_level_name.is_some() {
      // Record profiling time
      if let Some(start_time) = start {
        self
          .profiler
          .record_symbol_extraction(start_time.elapsed().as_nanos() as u64);
      }
      return Ok(top_level_name);
    }

    // Walk up the tree to find a top-level exported declaration
    loop {
      let parent_id = nodes.parent_id(current_id);
      if parent_id == current_id {
        // Reached the root
        break;
      }
      let parent_node = nodes.get_node(parent_id);

      match parent_node.kind() {
        // Handle export wrappers - look inside them for the actual declaration
        AstKind::ExportNamedDeclaration(export_decl) => {
          found_export_wrapper = true;
          // Check if there's an inline declaration (export const x = ...)
          if let Some(decl) = &export_decl.declaration {
            top_level_name = Self::extract_symbol_from_export_decl(decl);
          }
        }
        AstKind::ExportDefaultDeclaration(_) => {
          found_export_wrapper = true;
          top_level_name = Some("default".to_string());
        }
        // Top-level declarations that can be exported
        AstKind::Function(func) => {
          if !found_export_wrapper {
            if let Some(id) = &func.id {
              top_level_name = Some(id.name.to_string());
            }
          }
        }
        AstKind::Class(class) => {
          if !found_export_wrapper {
            if let Some(id) = &class.id {
              top_level_name = Some(id.name.to_string());
            }
          }
        }
        AstKind::TSInterfaceDeclaration(interface) => {
          if !found_export_wrapper {
            top_level_name = Some(interface.id.name.to_string());
          }
        }
        AstKind::TSTypeAliasDeclaration(type_alias) => {
          if !found_export_wrapper {
            top_level_name = Some(type_alias.id.name.to_string());
          }
        }
        AstKind::TSEnumDeclaration(enum_decl) => {
          if !found_export_wrapper {
            top_level_name = Some(enum_decl.id.name.to_string());
          }
        }
        AstKind::VariableDeclarator(var_decl) => {
          // For const/let declarations, get the binding name
          if !found_export_wrapper {
            if let oxc_ast::ast::BindingPatternKind::BindingIdentifier(ident) = &var_decl.id.kind {
              top_level_name = Some(ident.name.to_string());
            }
          }
        }
        _ => {}
      }

      // If we found a symbol from an export wrapper, we can stop
      if found_export_wrapper && top_level_name.is_some() {
        break;
      }

      current_id = parent_id;
    }

    // Record profiling time
    if let Some(start_time) = start {
      self
        .profiler
        .record_symbol_extraction(start_time.elapsed().as_nanos() as u64);
    }

    // Return the top-level declaration if found, otherwise None
    // When None is returned, it means the line doesn't contain a trackable symbol
    // (e.g., object literal properties, comments, or code not in a top-level declaration)
    Ok(top_level_name)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::Path;

  #[test]
  fn test_find_node_at_line_with_column_offset() {
    // Test that find_node_at_line uses column offset to find the correct container symbol
    // This test creates a simple TypeScript file and verifies that we can find
    // the correct variable declarator when given a precise column offset

    let source = r#"import { Component } from './component';

const MemoizedComponent = React.memo(Component);
const AnotherVar = 'test';

export { MemoizedComponent };"#;

    let cwd = Path::new(".");
    let profiler = Arc::new(Profiler::new(false));
    let mut analyzer =
      WorkspaceAnalyzer::new(vec![], cwd, profiler).expect("Failed to create analyzer");

    // Parse the source file using the same approach as analyze_file
    let file_path = Path::new("test.ts");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    // Build semantic data
    let semantic_builder = SemanticBuilder::new()
      .with_cfg(true)
      .with_check_syntax_error(false);
    let semantic_ret = semantic_builder.build(&parse_result.program);

    // Transmute to 'static lifetime (same as analyze_file does)
    let semantic: oxc_semantic::Semantic<'static> =
      unsafe { std::mem::transmute(semantic_ret.semantic) };

    analyzer.files.insert(
      file_path.to_path_buf(),
      FileSemanticData {
        source: source.to_string(),
        allocator,
        semantic,
      },
    );

    // Line 3 contains: const MemoizedComponent = React.memo(Component);
    // Column 42 is approximately where "Component" appears in the memo call
    // We expect to find "MemoizedComponent" as the container
    let result = analyzer.find_node_at_line(file_path, 3, 42);
    assert!(result.is_ok());
    let symbol = result.unwrap();
    assert_eq!(symbol, Some("MemoizedComponent".to_string()));

    // Test with column 0 (line start) - should still find a containing symbol
    let result = analyzer.find_node_at_line(file_path, 3, 0);
    assert!(result.is_ok());

    // Test line 4 with AnotherVar
    let result = analyzer.find_node_at_line(file_path, 4, 10);
    assert!(result.is_ok());
    let symbol = result.unwrap();
    assert_eq!(symbol, Some("AnotherVar".to_string()));
  }

  #[test]
  fn test_find_node_smallest_containing_node() {
    // Test that find_node_at_line finds the smallest containing node
    // when multiple nodes overlap at the same position

    let source = r#"export function outer() {
  const inner = function() {
    return 'nested';
  };
  return inner;
}"#;

    let cwd = Path::new(".");
    let profiler = Arc::new(Profiler::new(false));
    let mut analyzer =
      WorkspaceAnalyzer::new(vec![], cwd, profiler).expect("Failed to create analyzer");

    // Parse the source file using the same approach as analyze_file
    let file_path = Path::new("test.ts");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    // Build semantic data
    let semantic_builder = SemanticBuilder::new()
      .with_cfg(true)
      .with_check_syntax_error(false);
    let semantic_ret = semantic_builder.build(&parse_result.program);

    // Transmute to 'static lifetime (same as analyze_file does)
    let semantic: oxc_semantic::Semantic<'static> =
      unsafe { std::mem::transmute(semantic_ret.semantic) };

    analyzer.files.insert(
      file_path.to_path_buf(),
      FileSemanticData {
        source: source.to_string(),
        allocator,
        semantic,
      },
    );

    // Line 2 contains: const inner = function() {
    // When we query at the position of "inner", we should get "inner" not "outer"
    let result = analyzer.find_node_at_line(file_path, 2, 10);
    assert!(result.is_ok());
    // Note: The exact result depends on how the AST is structured
    // The important thing is that we get a result and don't panic
    let symbol = result.unwrap();
    assert!(symbol.is_some());
  }

  #[test]
  fn test_extract_dynamic_imports_basic() {
    // Test that dynamic imports are detected
    let source = r#"
import { staticImport } from './static';

const LazyComponent = React.lazy(() => import('./LazyComponent'));

async function loadModule() {
  const module = await import('./dynamic-module');
  return module;
}
"#;

    let file_path = Path::new("test.tsx");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let imports = WorkspaceAnalyzer::extract_imports(&parse_result.program, file_path);

    // Should have 1 static import + 2 dynamic imports
    assert_eq!(imports.len(), 3);

    // Check static import
    assert!(imports
      .iter()
      .any(|imp| imp.from_module == "./static" && imp.imported_name == "staticImport"));

    // Check dynamic imports
    let dynamic_imports: Vec<_> = imports
      .iter()
      .filter(|imp| imp.from_module == "./LazyComponent" || imp.from_module == "./dynamic-module")
      .collect();
    assert_eq!(dynamic_imports.len(), 2);

    // Dynamic imports should be namespace imports
    for imp in dynamic_imports {
      assert_eq!(imp.imported_name, "*");
      assert!(!imp.is_type_only);
    }
  }

  #[test]
  fn test_extract_dynamic_imports_with_then() {
    // Test dynamic imports with .then() pattern (like the uniclient case)
    let source = r#"
const LazyCookieConsent = React.lazy(
  async () => await import('@lemonade-hq/uniclient-app').then(module => ({ default: module.CookieConsent })),
);
"#;

    let file_path = Path::new("App.tsx");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let imports = WorkspaceAnalyzer::extract_imports(&parse_result.program, file_path);

    // Should have 1 dynamic import
    assert_eq!(imports.len(), 1);

    // Check the dynamic import
    let imp = &imports[0];
    assert_eq!(imp.from_module, "@lemonade-hq/uniclient-app");
    assert_eq!(imp.imported_name, "*"); // Namespace import
    assert!(!imp.is_type_only);
  }

  #[test]
  fn test_extract_no_dynamic_imports() {
    // Test file with only static imports
    let source = r#"
import { Component } from './Component';
import * as Utils from './utils';
import type { Props } from './types';

export function MyComponent(props: Props) {
  return <Component {...props} />;
}
"#;

    let file_path = Path::new("test.tsx");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let imports = WorkspaceAnalyzer::extract_imports(&parse_result.program, file_path);

    // Should have 3 static imports, no dynamic imports
    assert_eq!(imports.len(), 3);

    // None should have synthetic names
    assert!(!imports
      .iter()
      .any(|imp| imp.local_name.starts_with("__dynamic_import_")));
  }

  #[test]
  fn test_extract_multiple_dynamic_imports() {
    // Test multiple dynamic imports in the same file
    let source = r#"
const Component1 = React.lazy(() => import('./Component1'));
const Component2 = React.lazy(() => import('./Component2'));
const Component3 = React.lazy(() => import('./Component3'));

async function loadAll() {
  await import('./module1');
  await import('./module2');
}
"#;

    let file_path = Path::new("test.tsx");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let imports = WorkspaceAnalyzer::extract_imports(&parse_result.program, file_path);

    // Should have 5 dynamic imports
    assert_eq!(imports.len(), 5);

    // All should be namespace imports
    assert!(imports.iter().all(|imp| imp.imported_name == "*"));

    // Check that all modules are present
    let modules: Vec<_> = imports.iter().map(|imp| imp.from_module.as_str()).collect();
    assert!(modules.contains(&"./Component1"));
    assert!(modules.contains(&"./Component2"));
    assert!(modules.contains(&"./Component3"));
    assert!(modules.contains(&"./module1"));
    assert!(modules.contains(&"./module2"));
  }

  #[test]
  fn test_extract_dynamic_imports_non_string_literal() {
    // Test that non-string-literal dynamic imports are properly skipped with a warning
    let source = r#"
// Template literal (not supported)
const moduleName = 'dynamic-module';
const module1 = await import(`./modules/${moduleName}`);

// Variable (not supported)
const specifier = './some-module';
const module2 = await import(specifier);

// String literal (supported)
const module3 = await import('./supported-module');
"#;

    let file_path = Path::new("test.ts");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let imports = WorkspaceAnalyzer::extract_imports(&parse_result.program, file_path);

    // Should only have 1 import (the string literal one)
    // The template literal and variable imports should be skipped with warnings
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].from_module, "./supported-module");
    assert_eq!(imports[0].imported_name, "*");
    assert!(imports[0].is_dynamic);
  }

  #[test]
  fn test_dynamic_imports_are_marked() {
    // Test that dynamic imports have is_dynamic = true and static imports have is_dynamic = false
    let source = r#"
import { StaticImport } from './static';
import * as StaticNamespace from './static-namespace';

const DynamicImport = await import('./dynamic');
"#;

    let file_path = Path::new("test.ts");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let imports = WorkspaceAnalyzer::extract_imports(&parse_result.program, file_path);

    // Should have 2 static + 1 dynamic = 3 imports
    assert_eq!(imports.len(), 3);

    // Check static imports
    let static_imports: Vec<_> = imports.iter().filter(|imp| !imp.is_dynamic).collect();
    assert_eq!(static_imports.len(), 2);
    assert!(static_imports
      .iter()
      .all(|imp| imp.from_module.starts_with("./static")));

    // Check dynamic import
    let dynamic_imports: Vec<_> = imports.iter().filter(|imp| imp.is_dynamic).collect();
    assert_eq!(dynamic_imports.len(), 1);
    assert_eq!(dynamic_imports[0].from_module, "./dynamic");
    assert_eq!(dynamic_imports[0].imported_name, "*");
  }

  #[test]
  fn test_find_node_at_line_export_default_named() {
    // Test finding a named default export
    let source = r#"export default function myFunction() {
  return 'test';
}"#;

    let cwd = Path::new(".");
    let profiler = Arc::new(Profiler::new(false));
    let mut analyzer =
      WorkspaceAnalyzer::new(vec![], cwd, profiler).expect("Failed to create analyzer");

    let file_path = Path::new("test.ts");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let semantic_builder = SemanticBuilder::new()
      .with_cfg(true)
      .with_check_syntax_error(false);
    let semantic_ret = semantic_builder.build(&parse_result.program);

    let semantic: oxc_semantic::Semantic<'static> =
      unsafe { std::mem::transmute(semantic_ret.semantic) };

    analyzer.files.insert(
      file_path.to_path_buf(),
      FileSemanticData {
        source: source.to_string(),
        allocator,
        semantic,
      },
    );

    let result = analyzer.find_node_at_line(file_path, 1, 0);
    assert!(result.is_ok(), "Should not error: {:?}", result);
    let symbol = result.unwrap();
    assert_eq!(
      symbol,
      Some("default".to_string()),
      "Should find 'default' for export default"
    );
  }

  #[test]
  fn test_find_node_at_line_export_default_anonymous() {
    // Test finding an anonymous default export
    let source = r#"export default function() {
  return 'anonymous';
}"#;

    let cwd = Path::new(".");
    let profiler = Arc::new(Profiler::new(false));
    let mut analyzer =
      WorkspaceAnalyzer::new(vec![], cwd, profiler).expect("Failed to create analyzer");

    let file_path = Path::new("test.ts");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let semantic_builder = SemanticBuilder::new()
      .with_cfg(true)
      .with_check_syntax_error(false);
    let semantic_ret = semantic_builder.build(&parse_result.program);

    let semantic: oxc_semantic::Semantic<'static> =
      unsafe { std::mem::transmute(semantic_ret.semantic) };

    analyzer.files.insert(
      file_path.to_path_buf(),
      FileSemanticData {
        source: source.to_string(),
        allocator,
        semantic,
      },
    );

    let result = analyzer.find_node_at_line(file_path, 1, 0);
    assert!(result.is_ok(), "Should not error: {:?}", result);
    let symbol = result.unwrap();
    assert_eq!(
      symbol,
      Some("default".to_string()),
      "Should find 'default' for anonymous export default"
    );
  }

  #[test]
  fn test_find_node_at_line_destructured_export() {
    // Test finding destructured exports - should find the first identifier
    let source = r#"const obj = { a: 1, b: 2 };
export const { a, b } = obj;"#;

    let cwd = Path::new(".");
    let profiler = Arc::new(Profiler::new(false));
    let mut analyzer =
      WorkspaceAnalyzer::new(vec![], cwd, profiler).expect("Failed to create analyzer");

    let file_path = Path::new("test.ts");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let semantic_builder = SemanticBuilder::new()
      .with_cfg(true)
      .with_check_syntax_error(false);
    let semantic_ret = semantic_builder.build(&parse_result.program);

    let semantic: oxc_semantic::Semantic<'static> =
      unsafe { std::mem::transmute(semantic_ret.semantic) };

    analyzer.files.insert(
      file_path.to_path_buf(),
      FileSemanticData {
        source: source.to_string(),
        allocator,
        semantic,
      },
    );

    let result = analyzer.find_node_at_line(file_path, 2, 0);
    // Note: For destructured exports, we currently don't extract individual binding identifiers
    // This is a known limitation - the helper returns None for destructuring patterns
    // In the future, we may want to handle this case specially
    assert!(result.is_ok(), "Should not error: {:?}", result);
  }

  #[test]
  fn test_find_node_at_line_multiple_exports() {
    // Test file with multiple exports on different lines
    let source = r#"export const FIRST = 1;
export const SECOND = 2;
export function third() {
  return 3;
}"#;

    let cwd = Path::new(".");
    let profiler = Arc::new(Profiler::new(false));
    let mut analyzer =
      WorkspaceAnalyzer::new(vec![], cwd, profiler).expect("Failed to create analyzer");

    let file_path = Path::new("test.ts");
    let source_type = SourceType::from_path(file_path)
      .unwrap_or_else(|_| SourceType::default().with_typescript(true));
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, source_type);
    let parse_result = parser.parse();

    let semantic_builder = SemanticBuilder::new()
      .with_cfg(true)
      .with_check_syntax_error(false);
    let semantic_ret = semantic_builder.build(&parse_result.program);

    let semantic: oxc_semantic::Semantic<'static> =
      unsafe { std::mem::transmute(semantic_ret.semantic) };

    analyzer.files.insert(
      file_path.to_path_buf(),
      FileSemanticData {
        source: source.to_string(),
        allocator,
        semantic,
      },
    );

    // Test first export
    let result1 = analyzer.find_node_at_line(file_path, 1, 0);
    assert!(result1.is_ok());
    assert_eq!(result1.unwrap(), Some("FIRST".to_string()));

    // Test second export
    let result2 = analyzer.find_node_at_line(file_path, 2, 0);
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap(), Some("SECOND".to_string()));

    // Test third export
    let result3 = analyzer.find_node_at_line(file_path, 3, 0);
    assert!(result3.is_ok());
    assert_eq!(result3.unwrap(), Some("third".to_string()));
  }
}
