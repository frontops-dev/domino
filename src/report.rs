use crate::error::Result;
use crate::types::{AffectCause, AffectedProjectInfo, AffectedReport};
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
  let banner_html = generate_global_banner_html(report);
  let metadata_script = generate_metadata_script(report);
  let total_causes = report
    .projects
    .iter()
    .map(|p| p.causes.len())
    .sum::<usize>();
  // Render the global/semantic split when global invalidation happened so
  // the user can tell what came from `nx affected`-style global rules vs.
  // real semantic signal. Hidden by `display: none` otherwise.
  let summary_split_style = if report.global_triggers.is_empty() {
    "display: none;"
  } else {
    ""
  };

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

        /* Distinct pill for Nx namedInputs global invalidation. Deliberately
           a neutral slate so the user reads it as "infrastructure rule fired"
           rather than "real direct code change" (which uses .direct/green). */
        .cause-type.global {{
            background: #64748b;
        }}

        .global-banner {{
            background: linear-gradient(135deg, #1e293b 0%, #334155 100%);
            border: 1px solid #475569;
            border-left: 4px solid #64748b;
            border-radius: 8px;
            padding: 1.25rem 1.5rem;
            margin-bottom: 1.5rem;
            color: #e2e8f0;
        }}

        .global-banner h3 {{
            margin: 0 0 0.5rem 0;
            color: #f1f5f9;
            font-size: 1.1rem;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}

        .global-banner p {{
            margin: 0.5rem 0;
            color: #cbd5e1;
            font-size: 0.95rem;
            line-height: 1.5;
        }}

        .global-banner ul {{
            margin: 0.75rem 0 0.5rem 0;
            padding-left: 1.5rem;
            color: #e2e8f0;
        }}

        .global-banner ul li {{
            margin: 0.25rem 0;
            font-size: 0.9rem;
        }}

        .global-banner .docs-link {{
            color: #93c5fd;
            text-decoration: none;
            border-bottom: 1px dotted #93c5fd;
        }}

        .global-banner .docs-link:hover {{
            color: #bfdbfe;
        }}

        .global-only-group {{
            margin-top: 2rem;
            background: #1a1a1a;
            border: 1px solid #2a2a2a;
            border-left: 4px solid #64748b;
            border-radius: 8px;
            padding: 1rem 1.25rem;
        }}

        .global-only-group > summary {{
            cursor: pointer;
            list-style: none;
            color: #cbd5e1;
            font-weight: 600;
            font-size: 1rem;
            user-select: none;
        }}

        .global-only-group > summary::-webkit-details-marker {{
            display: none;
        }}

        .global-only-group > summary::before {{
            content: '▸';
            display: inline-block;
            margin-right: 0.5rem;
            transition: transform 0.15s ease;
        }}

        .global-only-group[open] > summary::before {{
            transform: rotate(90deg);
        }}

        .global-only-group .global-only-inner {{
            margin-top: 1rem;
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
    {}
    <div class="container">
        <header>
            <h1>🎯 True Affected Report</h1>
            <p class="subtitle">Dependency Graph & Impact Analysis</p>
        </header>

        {}

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
                <div class="summary-item" style="{}">
                    <div class="summary-value">{}</div>
                    <div class="summary-label">Globally Invalidated</div>
                </div>
                <div class="summary-item" style="{}">
                    <div class="summary-value">{}</div>
                    <div class="summary-label">Semantically Affected</div>
                </div>
                <div class="summary-item">
                    <div class="summary-value">{}</div>
                    <div class="summary-label">Changed Files</div>
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
            Generated by <strong>domino</strong> v{} &middot; True Affected Analysis
        </footer>
    </div>
</body>
</html>"#,
    graph_data,
    metadata_script,
    banner_html,
    format_number(report.projects.len()),
    format_number(total_causes),
    summary_split_style,
    format_number(report.totals.globally_invalidated),
    summary_split_style,
    format_number(report.totals.semantically_affected),
    format_number(report.totals.changed_files),
    details_html,
    env!("CARGO_PKG_VERSION"),
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
        AffectCause::AssetChange { .. } => {
          direct_changes.insert(project.name.clone());
        }
        AffectCause::LockfileChange { .. } => {
          direct_changes.insert(project.name.clone());
        }
        AffectCause::GlobalInvalidation { .. } => {
          direct_changes.insert(project.name.clone());
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

/// Render the "Global invalidation detected" banner that explains *why* a
/// whole-workspace affected list isn't a Domino misbehavior. Returns an
/// empty string for non-global runs so the template stays compact.
fn generate_global_banner_html(report: &AffectedReport) -> String {
  if report.global_triggers.is_empty() {
    return String::new();
  }

  let project_count = report.projects.len();
  let trigger_count = report.global_triggers.len();

  let mut items = String::new();
  for trigger in &report.global_triggers {
    items.push_str(&format!(
      "<li><code>{}</code> &nbsp;&larr;&nbsp; matched <code>{}</code></li>",
      html_escape(&trigger.file.display().to_string()),
      html_escape(&trigger.named_input),
    ));
  }

  format!(
    r#"<div class="global-banner" role="status">
            <h3>⚠️ Global invalidation detected</h3>
            <p>
                <strong>{trigger_count}</strong> changed file{trigger_plural} matched Nx <code>namedInputs</code>
                workspace-root pattern{trigger_plural}, so all <strong>{project_count}</strong> projects
                are marked affected &mdash; the same as <code>nx affected</code> would do.
            </p>
            <ul>{items}</ul>
            <p>
                Semantic analysis of the other changed files still ran &mdash; the
                <em>Detailed Impact Analysis</em> section below separates projects affected by
                real code signal from those only swept up by the global rule.
                See the
                <a class="docs-link" href="https://nx.dev/concepts/more-concepts/customizing-inputs" target="_blank" rel="noopener">
                    Nx <code>namedInputs</code> docs
                </a>
                for the underlying mechanic.
            </p>
        </div>"#,
    trigger_count = trigger_count,
    trigger_plural = if trigger_count == 1 { "" } else { "s" },
    project_count = format_number(project_count),
    items = items,
  )
}

/// Embed the run's machine-readable summary so downstream consumers (CI
/// dashboards, scrapers) don't have to parse the rendered HTML.
fn generate_metadata_script(report: &AffectedReport) -> String {
  // serde_json handles all string escaping for us — never hand-roll JSON into
  // an HTML attribute, and never use raw user-provided strings without escaping
  // `</script>` (the JSON shape here has no controlled-by-attacker strings, but
  // the safe default is to use serde_json::to_string which won't emit unescaped
  // `<` either since these values are paths and identifiers).
  match serde_json::to_string(report) {
    Ok(json) => format!(
      r#"<script type="application/json" id="domino-meta">{}</script>"#,
      json
    ),
    Err(_) => String::new(),
  }
}

fn html_escape(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
    .replace('\'', "&#39;")
}

/// Build the suffix shown in the collapsed group's summary line. When every
/// globally-invalidated project was hit by patterns from a single namedInput,
/// surface that name (e.g. " via `sharedGlobals`"). Otherwise stay generic.
fn global_only_group_label(global_only: &[&AffectedProjectInfo]) -> String {
  use std::collections::HashSet;
  let mut inputs: HashSet<&str> = HashSet::new();
  for project in global_only {
    for cause in &project.causes {
      if let AffectCause::GlobalInvalidation { named_input, .. } = cause {
        inputs.insert(named_input.as_str());
      }
    }
  }
  match inputs.len() {
    1 => {
      let name = inputs.into_iter().next().unwrap_or("");
      format!(" via <code>{}</code>", html_escape(name))
    }
    _ => " via Nx <code>namedInputs</code>".to_string(),
  }
}

/// Returns true when this project's only signal is global invalidation.
/// Used to collapse the long tail at the bottom of the report so semantic
/// signal isn't drowned out.
fn is_globally_invalidated_only(project: &AffectedProjectInfo) -> bool {
  !project.causes.is_empty()
    && project
      .causes
      .iter()
      .all(|c| matches!(c, AffectCause::GlobalInvalidation { .. }))
}

fn generate_details_html(report: &AffectedReport) -> String {
  let mut html = String::new();

  // Partition projects so semantic signal (DirectChange, ImportedSymbol, etc.)
  // floats to the top expanded, and the global-only long tail is collapsed
  // into a single summary row at the bottom.
  let (global_only, semantic): (Vec<_>, Vec<_>) = report
    .projects
    .iter()
    .partition(|p| is_globally_invalidated_only(p));

  for project in &semantic {
    // Determine affect type
    let mut has_direct = false;
    let mut has_imported = false;

    for cause in &project.causes {
      match cause {
        AffectCause::DirectChange { .. }
        | AffectCause::AssetChange { .. }
        | AffectCause::LockfileChange { .. }
        | AffectCause::GlobalInvalidation { .. } => has_direct = true,
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
        AffectCause::AssetChange {
          asset_file,
          referenced_in,
          line,
        } => {
          html.push_str("<span class=\"cause-type direct\">Asset Change</span>");
          html.push_str("<div class=\"cause-details\">");
          html.push_str(&format!(
            "Asset: <span class=\"code-path\">{}</span><br/>",
            asset_file.display()
          ));
          html.push_str(&format!(
            "Referenced in: <span class=\"code-path\">{}</span> (line {})",
            referenced_in.display(),
            line
          ));
          html.push_str("</div>");
        }
        AffectCause::LockfileChange {
          dependency,
          importing_file,
        } => {
          html.push_str("<span class=\"cause-type direct\">Lockfile Change</span>");
          html.push_str("<div class=\"cause-details\">");
          html.push_str(&format!(
            "Dependency: <span class=\"symbol\">{}</span><br/>",
            dependency
          ));
          html.push_str(&format!(
            "Imported in: <span class=\"code-path\">{}</span>",
            importing_file.display()
          ));
          html.push_str("</div>");
        }
        AffectCause::GlobalInvalidation { file, named_input } => {
          html.push_str("<span class=\"cause-type global\">Global Invalidation</span>");
          html.push_str("<div class=\"cause-details\">");
          html.push_str(&format!(
            "Triggered by: <span class=\"code-path\">{}</span> &nbsp;&larr;&nbsp; matched <code>{}</code>",
            html_escape(&file.display().to_string()),
            html_escape(named_input),
          ));
          html.push_str("</div>");
        }
      }

      html.push_str("</li>");
    }

    html.push_str("</ul></div></details></div>");
  }

  // Collapsed group for projects that are only on the affected list because
  // of global invalidation. Hidden by default to keep semantic signal visible.
  if !global_only.is_empty() {
    let group_label = global_only_group_label(&global_only);
    html.push_str(&format!(
      r#"<details class="global-only-group">
                <summary>{count} projects globally invalidated{label} &mdash; click to expand</summary>
                <div class="global-only-inner">"#,
      count = format_number(global_only.len()),
      label = group_label,
    ));

    for project in &global_only {
      html.push_str(&format!(
        r#"<div class="project-card" data-filter-type="affected">
                    <details>
                        <summary>
                            <div class="project-name">📦 {name}</div>
                            <div class="badge-container">
                                <span class="affect-badge badge-affected">Global Only</span>
                                <span class="affect-badge" style="background: #555;">
                                    {count} cause{plural}
                                </span>
                            </div>
                        </summary>
                        <div class="cause-list-container">
                            <ul class="cause-list">"#,
        name = html_escape(&project.name),
        count = project.causes.len(),
        plural = if project.causes.len() == 1 { "" } else { "s" },
      ));

      for cause in &project.causes {
        if let AffectCause::GlobalInvalidation { file, named_input } = cause {
          html.push_str(&format!(
            r#"<li class="cause-item">
                                <span class="cause-type global">Global Invalidation</span>
                                <div class="cause-details">
                                    Triggered by: <span class="code-path">{}</span> &nbsp;&larr;&nbsp; matched <code>{}</code>
                                </div>
                            </li>"#,
            html_escape(&file.display().to_string()),
            html_escape(named_input),
          ));
        }
      }

      html.push_str("</ul></div></details></div>");
    }

    html.push_str("</div></details>");
  }

  html
}

fn sanitize_node_id(name: &str) -> String {
  name.replace('-', "_").replace('@', "").replace('/', "_")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::types::{AffectedProjectInfo, GlobalTrigger, ReportTotals};
  use std::path::PathBuf;

  fn make_project(name: &str, causes: Vec<AffectCause>) -> AffectedProjectInfo {
    AffectedProjectInfo {
      name: name.to_string(),
      causes,
    }
  }

  fn empty_totals() -> ReportTotals {
    ReportTotals::default()
  }

  fn synth_global_report() -> AffectedReport {
    // Mimics the real-world incident: ~600 projects swept up by a workflow
    // file change, ~15 with semantic signal, 3 global trigger files.
    let mut projects = Vec::new();
    for i in 0..600 {
      projects.push(make_project(
        &format!("lib-{:03}", i),
        vec![
          AffectCause::GlobalInvalidation {
            file: PathBuf::from(".github/workflows/ci.yml"),
            named_input: "sharedGlobals".to_string(),
          },
          AffectCause::GlobalInvalidation {
            file: PathBuf::from("nx.json"),
            named_input: "sharedGlobals".to_string(),
          },
          AffectCause::GlobalInvalidation {
            file: PathBuf::from("package.json"),
            named_input: "ciInputs".to_string(),
          },
        ],
      ));
    }
    for i in 0..15 {
      projects.push(make_project(
        &format!("app-{}", i),
        vec![
          AffectCause::DirectChange {
            file: PathBuf::from(format!("apps/app-{}/src/index.ts", i)),
            symbol: Some(format!("handleClick{}", i)),
            line: 42 + i,
          },
          AffectCause::ImportedSymbol {
            source_project: "shared-utils".to_string(),
            symbol: "formatDate".to_string(),
            via_file: PathBuf::from(format!("apps/app-{}/src/index.ts", i)),
            source_file: PathBuf::from("libs/shared-utils/src/format.ts"),
          },
          AffectCause::GlobalInvalidation {
            file: PathBuf::from(".github/workflows/ci.yml"),
            named_input: "sharedGlobals".to_string(),
          },
        ],
      ));
    }
    projects.sort_by(|a, b| a.name.cmp(&b.name));

    AffectedReport {
      projects,
      global_triggers: vec![
        GlobalTrigger {
          file: PathBuf::from(".github/workflows/ci.yml"),
          named_input: "sharedGlobals".to_string(),
          raw_pattern: "{workspaceRoot}/.github/workflows/ci.yml".to_string(),
        },
        GlobalTrigger {
          file: PathBuf::from("nx.json"),
          named_input: "sharedGlobals".to_string(),
          raw_pattern: "{workspaceRoot}/nx.json".to_string(),
        },
        GlobalTrigger {
          file: PathBuf::from("package.json"),
          named_input: "ciInputs".to_string(),
          raw_pattern: "{workspaceRoot}/package.json".to_string(),
        },
      ],
      totals: ReportTotals {
        globally_invalidated: 600,
        semantically_affected: 15,
        overlap: 15,
        changed_files: 4,
      },
      version: env!("CARGO_PKG_VERSION"),
      run_started_at_unix_secs: 1_716_595_200,
    }
  }

  fn synth_normal_report() -> AffectedReport {
    let projects = vec![
      make_project(
        "app-web",
        vec![AffectCause::DirectChange {
          file: PathBuf::from("apps/web/src/main.ts"),
          symbol: Some("bootstrap".to_string()),
          line: 12,
        }],
      ),
      make_project(
        "shared-utils",
        vec![AffectCause::ImportedSymbol {
          source_project: "core".to_string(),
          symbol: "Logger".to_string(),
          via_file: PathBuf::from("libs/shared-utils/src/log.ts"),
          source_file: PathBuf::from("libs/core/src/logger.ts"),
        }],
      ),
      make_project(
        "ui-widgets",
        vec![AffectCause::ImplicitDependency {
          depends_on: "shared-utils".to_string(),
        }],
      ),
    ];
    AffectedReport {
      projects,
      global_triggers: Vec::new(),
      totals: ReportTotals {
        globally_invalidated: 0,
        semantically_affected: 3,
        overlap: 0,
        changed_files: 2,
      },
      version: env!("CARGO_PKG_VERSION"),
      run_started_at_unix_secs: 1_716_595_200,
    }
  }

  #[test]
  fn banner_renders_only_when_global_triggers_present() {
    let global = generate_html(&synth_global_report());
    let normal = generate_html(&synth_normal_report());

    assert!(global.contains("Global invalidation detected"));
    assert!(global.contains("cause-type global"));
    assert!(global.contains(r#"<div class="global-banner""#));
    assert!(global.contains(r#"id="domino-meta""#));

    // Non-global runs MUST remain visually identical to today's report —
    // no banner element, no new pill, no collapsed group element. (The CSS
    // *classes* are still emitted in the always-included <style> block; we
    // assert on the rendered elements, not the stylesheet.)
    assert!(!normal.contains("Global invalidation detected"));
    assert!(!normal.contains("\"cause-type global\""));
    assert!(!normal.contains(r#"<div class="global-banner""#));
    assert!(!normal.contains(r#"<details class="global-only-group""#));
    // The metadata script is harmless on non-global runs and lets dashboards
    // pick up the same shape regardless of run type.
    assert!(normal.contains(r#"id="domino-meta""#));
  }

  #[test]
  fn json_metadata_contains_camelcase_fields() {
    let html = generate_html(&synth_global_report());
    // Spot-check the camelCase shape so downstream consumers can rely on it.
    assert!(html.contains("\"globalTriggers\""));
    assert!(html.contains("\"namedInput\":\"sharedGlobals\""));
    assert!(html.contains("\"rawPattern\""));
    assert!(html.contains("\"runStartedAtUnixSecs\""));
    assert!(html.contains("\"globallyInvalidated\":600"));
  }

  #[test]
  fn global_only_projects_are_collapsed_at_bottom() {
    let html = generate_html(&synth_global_report());
    let semantic_start = html
      .find(r#"data-filter-type="both""#)
      .expect("expected a semantic project card with `both` filter type");
    let group_start = html
      .find(r#"<details class="global-only-group""#)
      .expect("expected the collapsed global-only group element");
    assert!(
      semantic_start < group_start,
      "semantic project cards must appear before the collapsed global-only group \
       (semantic at {}, group at {})",
      semantic_start,
      group_start
    );
  }

  #[test]
  fn pill_does_not_use_direct_class_for_global_cause() {
    // Regression guard for the original UX bug: the global cause was
    // sharing the green `.direct` pill, conflating it with real code edits.
    let html = generate_html(&synth_global_report());
    let first_global = html
      .find("Global Invalidation")
      .expect("global pill present");
    let nearby = &html[first_global.saturating_sub(80)..first_global];
    assert!(
      nearby.contains("cause-type global"),
      "expected `cause-type global` near the pill, got: {}",
      nearby
    );
  }

  // --- Sample report harness ---------------------------------------------
  //
  // These tests are #[ignore]d so they don't run in normal CI. Invoke with:
  //   cargo test --lib sample_report -- --ignored --nocapture
  // to write two HTML files you can open in a browser to sanity-check the UI.

  #[test]
  #[ignore]
  fn sample_report_global() {
    let html = generate_html(&synth_global_report());
    std::fs::write("/tmp/domino-report-global.html", &html).unwrap();
    println!(
      "wrote /tmp/domino-report-global.html ({} bytes)",
      html.len()
    );
  }

  #[test]
  #[ignore]
  fn sample_report_normal() {
    let html = generate_html(&synth_normal_report());
    std::fs::write("/tmp/domino-report-normal.html", &html).unwrap();
    println!(
      "wrote /tmp/domino-report-normal.html ({} bytes)",
      html.len()
    );
  }

  #[test]
  fn empty_report_does_not_panic() {
    // Defensive: AffectedReport with no projects shouldn't blow up.
    let report = AffectedReport {
      projects: Vec::new(),
      global_triggers: Vec::new(),
      totals: empty_totals(),
      version: env!("CARGO_PKG_VERSION"),
      run_started_at_unix_secs: 0,
    };
    let _ = generate_html(&report);
  }
}
