use crate::error::Result;
use crate::types::{AffectCause, AffectedReport};
use std::fs;
use std::path::Path;

/// Generate an interactive HTML report with a dependency graph
pub fn generate_html_report(report: &AffectedReport, output_path: &Path) -> Result<String> {
  let html = generate_html(report);
  fs::write(output_path, &html)?;
  Ok(html)
}

fn format_number(n: usize) -> String {
  let s = n.to_string();
  let mut result = String::new();
  let chars: Vec<char> = s.chars().collect();

  for (i, c) in chars.iter().enumerate() {
    if i > 0 && (chars.len() - i).is_multiple_of(3) {
      result.push(',');
    }
    result.push(*c);
  }

  result
}

fn generate_html(report: &AffectedReport) -> String {
  let graph_data = generate_cytoscape_data(report);
  let details_html = generate_details_html(report);
  let total_causes = report
    .projects
    .iter()
    .map(|p| p.causes.len())
    .sum::<usize>();

  format!(
    r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>True Affected - Dependency Report</title>
    <script defer src="https://unpkg.com/cytoscape@3.28.1/dist/cytoscape.min.js"></script>
    <script defer src="https://unpkg.com/dagre@0.8.5/dist/dagre.min.js"></script>
    <script defer src="https://unpkg.com/cytoscape-dagre@2.5.0/cytoscape-dagre.js"></script>
    <script defer src="https://unpkg.com/layout-base@2.0.1/layout-base.js"></script>
    <script defer src="https://unpkg.com/cose-base@2.2.0/cose-base.js"></script>
    <script defer src="https://unpkg.com/cytoscape-fcose@2.2.0/cytoscape-fcose.js"></script>
    <script defer src="https://unpkg.com/webcola@3.4.0/WebCola/cola.min.js"></script>
    <script defer src="https://unpkg.com/cytoscape-cola@2.5.1/cytoscape-cola.js"></script>
    <script defer src="https://unpkg.com/cytoscape-cose-bilkent@4.1.0/cytoscape-cose-bilkent.js"></script>
    <script>
        const graphData = {};
        let cy; // Make cy global for layout switching

        // Wait for all libraries to load
        function initGraph() {{
            // Check if all required libraries are loaded
            if (typeof cytoscape === 'undefined') {{
                return setTimeout(initGraph, 100);
            }}

            // Register layout extensions (check each one exists)
            if (typeof cytoscapeDagre !== 'undefined') {{
                cytoscape.use(cytoscapeDagre);
            }}
            if (typeof cytoscapeFcose !== 'undefined') {{
                cytoscape.use(cytoscapeFcose);
            }}
            if (typeof cytoscapeCola !== 'undefined') {{
                cytoscape.use(cytoscapeCola);
            }}
            if (typeof cytoscapeCoseBilkent !== 'undefined') {{
                cytoscape.use(cytoscapeCoseBilkent);
            }}

            cy = cytoscape({{
                container: document.getElementById('cy'),
                elements: graphData,
                style: [
                    {{
                        selector: 'node',
                        style: {{
                            'background-color': '#667eea',
                            'label': 'data(label)',
                            'color': '#fff',
                            'text-valign': 'center',
                            'text-halign': 'center',
                            'font-size': '12px',
                            'font-weight': '600',
                            'width': 'label',
                            'height': 'label',
                            'padding': '16px',
                            'shape': 'roundrectangle',
                            'text-wrap': 'wrap',
                            'text-max-width': '180px'
                        }}
                    }},
                    {{
                        selector: 'node[type="direct"]',
                        style: {{
                            'background-color': '#10b981',
                            'border-width': '3px',
                            'border-color': '#059669'
                        }}
                    }},
                    {{
                        selector: 'node[type="affected"]',
                        style: {{
                            'background-color': '#3b82f6',
                            'border-width': '2px',
                            'border-color': '#2563eb'
                        }}
                    }},
                    {{
                        selector: 'edge',
                        style: {{
                            'width': 2,
                            'line-color': '#667eea',
                            'target-arrow-color': '#667eea',
                            'target-arrow-shape': 'triangle',
                            'curve-style': 'bezier',
                            'label': 'data(label)',
                            'font-size': '10px',
                            'color': '#aaa',
                            'text-background-color': '#1a1a1a',
                            'text-background-opacity': 0.8,
                            'text-background-padding': '3px'
                        }}
                    }},
                    {{
                        selector: 'edge[type="implicit"]',
                        style: {{
                            'line-style': 'dashed',
                            'line-color': '#f59e0b',
                            'target-arrow-color': '#f59e0b'
                        }}
                    }}
                ],
                layout: {{
                    name: 'breadthfirst',
                    directed: true,
                    spacingFactor: 1.5,
                    animate: false,
                    fit: true,
                    padding: 30
                }},
                minZoom: 0.3,
                maxZoom: 3
            }});

            // Add tooltips
            cy.on('mouseover', 'node', function(evt) {{
                const node = evt.target;
                node.style('border-width', '4px');
            }});

            cy.on('mouseout', 'node', function(evt) {{
                const node = evt.target;
                const borderWidth = node.data('type') === 'direct' ? '3px' : '2px';
                node.style('border-width', borderWidth);
            }});

            // Fit to viewport
            cy.fit(50);
        }}

        // Initialize when DOM is ready
        if (document.readyState === 'loading') {{
            document.addEventListener('DOMContentLoaded', initGraph);
        }} else {{
            initGraph();
        }}

        function setActiveButton(btn) {{
            document.querySelectorAll('.layout-btn').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
        }}

        function switchLayout(layoutName) {{
            if (!cy) return;

            const layoutConfigs = {{
                'fcose': {{
                    name: 'fcose',
                    quality: 'default',
                    randomize: false,
                    animate: true,
                    animationDuration: 500,
                    fit: true,
                    padding: 30,
                    nodeSeparation: 75,
                    idealEdgeLength: 100,
                    edgeElasticity: 0.45,
                    nestingFactor: 0.1
                }},
                'dagre': {{
                    name: 'dagre',
                    rankDir: 'LR',
                    nodeSep: 50,
                    rankSep: 100,
                    padding: 20,
                    animate: true,
                    animationDuration: 500,
                    fit: true
                }},
                'cola': {{
                    name: 'cola',
                    animate: true,
                    animationDuration: 500,
                    fit: true,
                    padding: 30,
                    nodeDimensionsIncludeLabels: true,
                    edgeLength: 100,
                    nodeSpacing: 50
                }},
                'cose-bilkent': {{
                    name: 'cose-bilkent',
                    animate: true,
                    animationDuration: 500,
                    fit: true,
                    padding: 30,
                    nodeDimensionsIncludeLabels: true,
                    idealEdgeLength: 100,
                    nodeRepulsion: 4500,
                    edgeElasticity: 0.45
                }},
                'breadthfirst': {{
                    name: 'breadthfirst',
                    directed: true,
                    spacingFactor: 1.5,
                    animate: true,
                    animationDuration: 500,
                    fit: true,
                    padding: 30
                }},
                'circle': {{
                    name: 'circle',
                    animate: true,
                    animationDuration: 500,
                    fit: true,
                    padding: 30
                }},
                'concentric': {{
                    name: 'concentric',
                    animate: true,
                    animationDuration: 500,
                    fit: true,
                    padding: 30,
                    concentric: function(node) {{
                        return node.data('type') === 'direct' ? 2 : 1;
                    }},
                    levelWidth: function(nodes) {{
                        return 2;
                    }}
                }}
            }};

            const config = layoutConfigs[layoutName];
            if (config) {{
                cy.layout(config).run();
            }}
        }}

        function toggleAllDetails() {{
            const details = document.querySelectorAll('.project-card:not(.hidden) details');
            const btn = document.getElementById('toggleAllBtn');
            const anyOpen = Array.from(details).some(d => d.open);

            details.forEach(detail => {{
                detail.open = !anyOpen;
            }});

            btn.textContent = anyOpen ? '▼ Expand All' : '▲ Collapse All';
        }}

        function setActiveFilter(btn) {{
            document.querySelectorAll('.filter-btn').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
        }}

        function filterProjects(filterType) {{
            const cards = document.querySelectorAll('.project-card');

            cards.forEach(card => {{
                const cardType = card.getAttribute('data-filter-type');

                if (filterType === 'all') {{
                    card.classList.remove('hidden');
                }} else if (cardType === filterType) {{
                    card.classList.remove('hidden');
                }} else if (cardType === 'both' && (filterType === 'direct' || filterType === 'affected')) {{
                    // "both" projects should appear in both "direct" and "affected" filters
                    card.classList.remove('hidden');
                }} else {{
                    card.classList.add('hidden');
                }}
            }});
        }}
    </script>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}

        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            background: #1a1a1a;
            color: #e0e0e0;
            line-height: 1.6;
        }}

        .container {{
            max-width: 1400px;
            margin: 0 auto;
            padding: 2rem;
        }}

        header {{
            position: sticky;
            top: 0;
            z-index: 100;
            background: #1a1a1a;
            text-align: center;
            padding: 2rem 0 1rem 0;
            margin-bottom: 2rem;
            border-bottom: 1px solid #3a3a3a;
        }}

        h1 {{
            font-size: 2.5rem;
            font-weight: 700;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
            background-clip: text;
            margin-bottom: 0.5rem;
        }}

        .subtitle {{
            color: #888;
            font-size: 1.1rem;
        }}

        .summary {{
            background: #2a2a2a;
            border-radius: 12px;
            padding: 1.5rem;
            margin-bottom: 2rem;
            border: 1px solid #3a3a3a;
        }}

        .summary-grid {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1.5rem;
        }}

        .summary-item {{
            text-align: center;
        }}

        .summary-value {{
            font-size: 2rem;
            font-weight: 700;
            color: #667eea;
        }}

        .summary-label {{
            color: #888;
            font-size: 0.9rem;
            text-transform: uppercase;
            letter-spacing: 0.05em;
        }}

        .graph-container {{
            background: #2a2a2a;
            border-radius: 12px;
            padding: 2rem;
            margin-bottom: 2rem;
            border: 1px solid #3a3a3a;
        }}

        .graph-legend {{
            display: flex;
            gap: 2rem;
            margin-bottom: 1.5rem;
            padding: 1rem;
            background: #1a1a1a;
            border-radius: 8px;
            border: 1px solid #3a3a3a;
            flex-wrap: wrap;
        }}

        .legend-item {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
            font-size: 0.85rem;
            color: #aaa;
        }}

        .legend-icon {{
            width: 32px;
            height: 32px;
            border-radius: 6px;
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 1rem;
        }}

        .legend-icon.direct {{
            background: #10b981;
            border: 3px solid #059669;
        }}

        .legend-icon.affected {{
            background: #3b82f6;
            border: 2px solid #2563eb;
        }}

        .legend-line {{
            width: 40px;
            height: 2px;
            position: relative;
        }}

        .legend-line.normal {{
            background: #667eea;
        }}

        .legend-line.normal::after {{
            content: '▶';
            position: absolute;
            right: -8px;
            top: -7px;
            color: #667eea;
            font-size: 12px;
        }}

        .legend-line.implicit {{
            background: #f59e0b;
            border-top: 2px dashed #f59e0b;
        }}

        .legend-line.implicit::after {{
            content: '▶';
            position: absolute;
            right: -8px;
            top: -9px;
            color: #f59e0b;
            font-size: 12px;
        }}

        #cy {{
            width: 100%;
            height: 600px;
            background: #1a1a1a;
            border-radius: 8px;
        }}

        .layout-controls {{
            display: flex;
            align-items: center;
            gap: 1rem;
            margin-bottom: 1rem;
            padding: 1rem;
            background: #1a1a1a;
            border-radius: 8px;
            border: 1px solid #3a3a3a;
        }}

        .layout-label {{
            color: #888;
            font-size: 0.9rem;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.05em;
        }}

        .layout-buttons {{
            display: flex;
            gap: 0.5rem;
            flex-wrap: wrap;
        }}

        .layout-btn {{
            background: #2a2a2a;
            color: #e0e0e0;
            border: 1px solid #3a3a3a;
            padding: 0.5rem 1rem;
            border-radius: 6px;
            font-size: 0.85rem;
            font-weight: 500;
            cursor: pointer;
            transition: all 0.2s;
        }}

        .layout-btn:hover {{
            background: #3a3a3a;
            border-color: #667eea;
            transform: translateY(-1px);
        }}

        .layout-btn.active {{
            background: #667eea;
            border-color: #667eea;
            color: #fff;
        }}

        .layout-btn:active {{
            transform: translateY(0);
        }}

        .details-container {{
            background: #2a2a2a;
            border-radius: 12px;
            padding: 2rem;
            border: 1px solid #3a3a3a;
        }}

        .filter-controls {{
            display: flex;
            gap: 0.5rem;
            margin-bottom: 1.5rem;
        }}

        .filter-btn {{
            background: #2a2a2a;
            color: #e0e0e0;
            border: 1px solid #3a3a3a;
            padding: 0.5rem 1rem;
            border-radius: 6px;
            font-size: 0.85rem;
            font-weight: 500;
            cursor: pointer;
            transition: all 0.2s;
        }}

        .filter-btn:hover {{
            background: #3a3a3a;
            border-color: #667eea;
        }}

        .filter-btn.active {{
            background: #667eea;
            border-color: #667eea;
            color: #fff;
        }}

        .project-card.hidden {{
            display: none;
        }}

        .project-card {{
            background: #222;
            border-radius: 8px;
            margin-bottom: 1rem;
            border-left: 4px solid #667eea;
        }}

        .project-card details {{
            cursor: pointer;
        }}

        .project-card summary {{
            padding: 1.5rem;
            list-style: none;
            display: flex;
            align-items: center;
            gap: 1rem;
            user-select: none;
        }}

        .project-card summary::-webkit-details-marker {{
            display: none;
        }}

        .project-card summary::before {{
            content: '▶';
            display: inline-block;
            width: 1em;
            transition: transform 0.2s;
            color: #667eea;
        }}

        .project-card details[open] summary::before {{
            transform: rotate(90deg);
        }}

        .project-name {{
            font-size: 1.3rem;
            font-weight: 600;
            color: #fff;
            flex: 1;
        }}

        .badge-container {{
            display: flex;
            gap: 0.5rem;
        }}

        .affect-badge {{
            padding: 0.35rem 0.75rem;
            border-radius: 12px;
            font-size: 0.75rem;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.05em;
        }}

        .badge-direct {{
            background: #10b981;
            color: #fff;
        }}

        .badge-affected {{
            background: #3b82f6;
            color: #fff;
        }}

        .badge-both {{
            background: linear-gradient(90deg, #10b981 0%, #3b82f6 100%);
            color: #fff;
        }}

        .cause-list-container {{
            padding: 0 1.5rem 1.5rem 1.5rem;
        }}

        .toggle-all-btn {{
            background: #667eea;
            color: #fff;
            border: none;
            padding: 0.75rem 1.5rem;
            border-radius: 8px;
            font-size: 0.95rem;
            font-weight: 600;
            cursor: pointer;
            margin-bottom: 1.5rem;
            transition: all 0.2s;
            display: inline-flex;
            align-items: center;
            gap: 0.5rem;
        }}

        .toggle-all-btn:hover {{
            background: #5a67d8;
            transform: translateY(-1px);
            box-shadow: 0 4px 12px rgba(102, 126, 234, 0.4);
        }}

        .toggle-all-btn:active {{
            transform: translateY(0);
        }}

        .cause-list {{
            list-style: none;
        }}

        .cause-item {{
            background: #1a1a1a;
            border-radius: 6px;
            padding: 1rem;
            margin-bottom: 0.75rem;
            border-left: 3px solid #444;
        }}

        .cause-type {{
            display: inline-block;
            background: #667eea;
            color: #fff;
            padding: 0.25rem 0.75rem;
            border-radius: 4px;
            font-size: 0.85rem;
            font-weight: 600;
            margin-bottom: 0.5rem;
        }}

        .cause-type.direct {{
            background: #10b981;
        }}

        .cause-type.imported {{
            background: #3b82f6;
        }}

        .cause-type.reexported {{
            background: #8b5cf6;
        }}

        .cause-type.implicit {{
            background: #f59e0b;
        }}

        .cause-details {{
            color: #aaa;
            font-size: 0.9rem;
            margin-top: 0.5rem;
        }}

        .code-path {{
            font-family: 'Monaco', 'Menlo', 'Courier New', monospace;
            background: #111;
            padding: 0.25rem 0.5rem;
            border-radius: 3px;
            color: #60a5fa;
            font-size: 0.85rem;
        }}

        .symbol {{
            font-family: 'Monaco', 'Menlo', 'Courier New', monospace;
            background: #111;
            padding: 0.25rem 0.5rem;
            border-radius: 3px;
            color: #a78bfa;
            font-size: 0.85rem;
        }}

        footer {{
            text-align: center;
            margin-top: 3rem;
            padding-top: 2rem;
            border-top: 1px solid #3a3a3a;
            color: #666;
        }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>🎯 True Affected Report</h1>
            <p class="subtitle">Dependency Graph & Impact Analysis</p>
        </header>

        <div class="summary">
            <div class="summary-grid">
                <div class="summary-item">
                    <div class="summary-value">{}</div>
                    <div class="summary-label">Affected Projects</div>
                </div>
                <div class="summary-item">
                    <div class="summary-value">{}</div>
                    <div class="summary-label">Total Causes</div>
                </div>
            </div>
        </div>

        <div class="graph-container">
            <h2 style="margin-bottom: 1.5rem; color: #fff;">Interactive Dependency Graph</h2>
            <p style="margin-bottom: 1rem; color: #888; font-size: 0.9rem;">
                💡 Pan, zoom, and drag nodes to explore • Hover over nodes for details
            </p>
            <div class="graph-legend">
                <div class="legend-item">
                    <div class="legend-icon direct">✏️</div>
                    <span>Direct Change</span>
                </div>
                <div class="legend-item">
                    <div class="legend-icon affected">📦</div>
                    <span>Affected Project</span>
                </div>
                <div class="legend-item">
                    <div class="legend-line normal"></div>
                    <span>Import Dependency</span>
                </div>
                <div class="legend-item">
                    <div class="legend-line implicit"></div>
                    <span>Implicit Dependency</span>
                </div>
            </div>
            <div class="layout-controls">
                <span class="layout-label">Layout:</span>
                <div class="layout-buttons">
                    <button class="layout-btn" onclick="switchLayout('fcose'); setActiveButton(this)">fCoSE</button>
                    <button class="layout-btn" onclick="switchLayout('dagre'); setActiveButton(this)">Dagre</button>
                    <button class="layout-btn" onclick="switchLayout('cola'); setActiveButton(this)">Cola</button>
                    <button class="layout-btn" onclick="switchLayout('cose-bilkent'); setActiveButton(this)">CoSE-Bilkent</button>
                    <button class="layout-btn active" onclick="switchLayout('breadthfirst'); setActiveButton(this)">Breadthfirst</button>
                    <button class="layout-btn" onclick="switchLayout('circle'); setActiveButton(this)">Circle</button>
                    <button class="layout-btn" onclick="switchLayout('concentric'); setActiveButton(this)">Concentric</button>
                </div>
            </div>
            <div id="cy"></div>
        </div>

        <div class="details-container">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
                <h2 style="color: #fff; margin: 0;">Detailed Impact Analysis</h2>
                <button id="toggleAllBtn" class="toggle-all-btn" onclick="toggleAllDetails()">
                    ▼ Expand All
                </button>
            </div>
            <div class="filter-controls">
                <button class="filter-btn active" onclick="filterProjects('all'); setActiveFilter(this)">All</button>
                <button class="filter-btn" onclick="filterProjects('direct'); setActiveFilter(this)">Direct</button>
                <button class="filter-btn" onclick="filterProjects('affected'); setActiveFilter(this)">Affected</button>
                <button class="filter-btn" onclick="filterProjects('both'); setActiveFilter(this)">Both</button>
            </div>
            {}
        </div>

        <footer>
            Generated by <strong>domino</strong> - True Affected Analysis
        </footer>
    </div>
</body>
</html>"#,
    graph_data,
    format_number(report.projects.len()),
    format_number(total_causes),
    details_html
  )
}

fn generate_cytoscape_data(report: &AffectedReport) -> String {
  use std::collections::{HashMap, HashSet};

  // Track project-to-project relationships
  let mut relationships: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
  let mut direct_changes: HashSet<String> = HashSet::new();

  // Analyze causes to build simplified graph
  for project in &report.projects {
    for cause in &project.causes {
      match cause {
        AffectCause::DirectChange { .. } => {
          direct_changes.insert(project.name.clone());
        }
        AffectCause::ImportedSymbol { source_project, .. } => {
          relationships
            .entry(source_project.clone())
            .or_default()
            .entry(project.name.clone())
            .or_default()
            .push("imported".to_string());
        }
        AffectCause::ReExported { .. } => {
          // Re-exports are internal to a project, don't show as separate edges
        }
        AffectCause::ImplicitDependency { depends_on } => {
          relationships
            .entry(depends_on.clone())
            .or_default()
            .entry(project.name.clone())
            .or_default()
            .push("implicit".to_string());
        }
      }
    }
  }

  // Build nodes array
  let mut nodes = Vec::new();
  for project in &report.projects {
    let node_type = if direct_changes.contains(&project.name) {
      "direct"
    } else {
      "affected"
    };

    let icon = if node_type == "direct" {
      "✏️ "
    } else {
      "📦 "
    };

    nodes.push(format!(
      r#"{{ data: {{ id: "{}", label: "{}{}", type: "{}" }} }}"#,
      sanitize_node_id(&project.name),
      icon,
      project.name,
      node_type
    ));
  }

  // Build edges array
  let mut edges = Vec::new();
  for (source, targets) in relationships {
    let source_id = sanitize_node_id(&source);
    for (target, cause_types) in targets {
      let target_id = sanitize_node_id(&target);

      // Count cause types
      let import_count = cause_types.iter().filter(|t| *t == "imported").count();
      let implicit_count = cause_types.iter().filter(|t| *t == "implicit").count();

      if implicit_count > 0 {
        edges.push(format!(
          r#"{{ data: {{ source: "{}", target: "{}", label: "implicit", type: "implicit" }} }}"#,
          source_id, target_id
        ));
      } else if import_count > 0 {
        let label = if import_count == 1 {
          "1 import".to_string()
        } else {
          format!("{} imports", import_count)
        };
        edges.push(format!(
          r#"{{ data: {{ source: "{}", target: "{}", label: "{}" }} }}"#,
          source_id, target_id, label
        ));
      }
    }
  }

  // Combine into Cytoscape JSON format
  format!(
    r#"{{ nodes: [{}], edges: [{}] }}"#,
    nodes.join(", "),
    edges.join(", ")
  )
}

fn generate_details_html(report: &AffectedReport) -> String {
  let mut html = String::new();

  for project in &report.projects {
    // Determine affect type
    let mut has_direct = false;
    let mut has_imported = false;

    for cause in &project.causes {
      match cause {
        AffectCause::DirectChange { .. } => has_direct = true,
        AffectCause::ImportedSymbol { .. } => has_imported = true,
        _ => {}
      }
    }

    let (badge, filter_type) = if has_direct && has_imported {
      (
        r#"<span class="affect-badge badge-both">Direct + Affected</span>"#,
        "both",
      )
    } else if has_direct {
      (
        r#"<span class="affect-badge badge-direct">Direct Change</span>"#,
        "direct",
      )
    } else {
      (
        r#"<span class="affect-badge badge-affected">Affected</span>"#,
        "affected",
      )
    };

    html.push_str(&format!(
      r#"<div class="project-card" data-filter-type="{}">
                <details>
                    <summary>
                        <div class="project-name">📦 {}</div>
                        <div class="badge-container">
                            {}
                            <span class="affect-badge" style="background: #555;">
                                {} cause{}
                            </span>
                        </div>
                    </summary>
                    <div class="cause-list-container">
                        <ul class="cause-list">
"#,
      filter_type,
      project.name,
      badge,
      project.causes.len(),
      if project.causes.len() == 1 { "" } else { "s" }
    ));

    for cause in &project.causes {
      html.push_str("<li class=\"cause-item\">");

      match cause {
        AffectCause::DirectChange { file, symbol, line } => {
          html.push_str("<span class=\"cause-type direct\">Direct Change</span>");
          html.push_str("<div class=\"cause-details\">");
          html.push_str(&format!(
            "File: <span class=\"code-path\">{}</span> (line {})",
            file.display(),
            line
          ));
          if let Some(sym) = symbol {
            html.push_str(&format!(
              "<br/>Symbol: <span class=\"symbol\">{}</span>",
              sym
            ));
          }
          html.push_str("</div>");
        }
        AffectCause::ImportedSymbol {
          source_project,
          symbol,
          via_file,
          source_file,
        } => {
          html.push_str("<span class=\"cause-type imported\">Imported Symbol</span>");
          html.push_str("<div class=\"cause-details\">");
          html.push_str(&format!(
            "Symbol: <span class=\"symbol\">{}</span><br/>",
            symbol
          ));
          html.push_str(&format!(
            "From project: <strong>{}</strong><br/>",
            source_project
          ));
          html.push_str(&format!(
            "Source: <span class=\"code-path\">{}</span><br/>",
            source_file.display()
          ));
          html.push_str(&format!(
            "Imported in: <span class=\"code-path\">{}</span>",
            via_file.display()
          ));
          html.push_str("</div>");
        }
        AffectCause::ReExported {
          through_file,
          symbol,
          source_file,
        } => {
          html.push_str("<span class=\"cause-type reexported\">Re-exported</span>");
          html.push_str("<div class=\"cause-details\">");
          html.push_str(&format!(
            "Symbol: <span class=\"symbol\">{}</span><br/>",
            symbol
          ));
          html.push_str(&format!(
            "Source: <span class=\"code-path\">{}</span><br/>",
            source_file.display()
          ));
          html.push_str(&format!(
            "Re-exported via: <span class=\"code-path\">{}</span>",
            through_file.display()
          ));
          html.push_str("</div>");
        }
        AffectCause::ImplicitDependency { depends_on } => {
          html.push_str("<span class=\"cause-type implicit\">Implicit Dependency</span>");
          html.push_str("<div class=\"cause-details\">");
          html.push_str(&format!("Depends on: <strong>{}</strong>", depends_on));
          html.push_str("</div>");
        }
      }

      html.push_str("</li>");
    }

    html.push_str("</ul></div></details></div>");
  }

  html
}

fn sanitize_node_id(name: &str) -> String {
  name.replace('-', "_").replace('@', "").replace('/', "_")
}
