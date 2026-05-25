use crate::error::Result;
use crate::types::{AffectCause, AffectedProjectInfo, AffectedReport};
use std::fs;
use std::path::Path;

/// Canonical project URL — surfaced in the top bar and footer so anyone with
/// the HTML in hand can get back to the source.
const REPO_URL: &str = "https://github.com/frontops-dev/domino";

/// Body of `docs/assets/logo.svg`, inlined so the report stays a single
/// self-contained HTML file. Keep in sync with the docs asset when the brand
/// mark changes — the file path is recorded here intentionally.
/// Source: docs/assets/logo.svg
const LOGO_SVG: &str = r##"<svg id="domino-logo" viewBox="0 0 200 200" fill="none" xmlns="http://www.w3.org/2000/svg" aria-label="domino logo" role="img"><path d="M78.3812 11.882C80.5232 4.5195 88.7579 -0.0415959 97.0989 2.93669L97.0999 2.93571L163.14 26.006L163.158 26.0119L163.176 26.0197C170.731 28.9903 174.235 37.4172 171.465 45.1545L122.279 188.194L122.28 188.195C119.857 195.504 112.473 200.07 104.54 197.667L104.518 197.66L104.496 197.653L36.1673 173.477L35.8372 173.36C28.9454 170.819 25.3949 163.324 27.7728 156.328L27.7767 156.32L78.3812 11.882Z" fill="url(#dlg0)" stroke="#959494" stroke-width="2"/><path d="M159.31 30.85L95.42 8.99001C90.43 7.23001 86.28 10.01 85.04 14.72L62 80.5L142 109L164.43 43.78C166.26 38.51 164.71 33.03 159.31 30.85Z" fill="#F26F0D"/><path d="M60.09 86.8L35.28 157.08C33.99 161.16 36.06 165.39 39.91 166.72L106.06 190.71C110.71 192.18 114.41 188.84 116.07 183.96L139.84 114.61L60.09 86.8Z" fill="url(#dlg1)"/><path d="M126.42 36.26L95.19 76.71L102.72 79.38L133.88 38.62L126.42 36.26Z" fill="#FEFFFE"/><path d="M119.96 118.27C115.28 118.06 112.96 121.67 113.27 126.09L91.72 135.79C89.92 133.81 87.21 132.87 85.46 133.73L74.61 111.84C77.92 108.87 76.98 102.28 71.32 100.24C65.43 98.97 62.48 104.62 63.41 108.77C64.42 112.98 68.12 114.58 71.36 113.59L82.68 134.92C80.84 136.42 80.04 138.33 80.13 140.83L57.82 150.37C55.84 147.62 51.01 146.74 47.81 149.55C44.12 153.19 45.63 158.89 49.99 160.93C54.92 162.66 59.52 158.82 59.16 154.12L81.02 144.28C82.89 147.09 86.04 147.25 88.08 146.34L99.18 167.46C95.06 170.99 96.05 176.15 100.41 178.75C105.44 180.87 109.71 176.96 110.12 173.22C110.31 168.88 106.48 165.69 102.04 166.06L91.22 144.73C93.05 143.48 93.62 140.84 93.19 138.6L114.21 129.41C116.82 133.01 122.61 132.78 125.22 129.6C128.48 125.77 126.41 119.19 119.96 118.27Z" fill="#B9E8F2"/><defs><linearGradient id="dlg0" x1="54.1908" y1="17.4036" x2="133.037" y2="181.855" gradientUnits="userSpaceOnUse"><stop stop-color="#505050"/><stop offset="1" stop-color="#272727"/></linearGradient><linearGradient id="dlg1" x1="67.6978" y1="91.8004" x2="107.257" y2="187.974" gradientUnits="userSpaceOnUse"><stop stop-color="#1C74AE"/><stop offset="1" stop-color="#125A8E"/></linearGradient></defs></svg>"##;

/// Lucide-style stroke icons. All use `stroke="currentColor"` so they recolor
/// via CSS. 16×16 viewport. Kept inline for the single-file constraint.
const ICON_GITHUB: &str = r##"<svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/></svg>"##;
const ICON_EXTERNAL: &str = r##"<svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 10L13 3M13 3H8M13 3v5M11 8.5v3a1.5 1.5 0 0 1-1.5 1.5h-6A1.5 1.5 0 0 1 2 11.5v-6A1.5 1.5 0 0 1 3.5 4h3"/></svg>"##;
const ICON_INFO: &str = r##"<svg viewBox="0 0 16 16" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="8" cy="8" r="6.5"/><path d="M8 7.5v3.5M8 5.25v.5"/></svg>"##;
const ICON_PACKAGE: &str = r##"<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 4.5L8 1.5 2 4.5v7L8 14.5l6-3v-7Z"/><path d="M2 4.5L8 7.5l6-3M8 7.5V14.5"/></svg>"##;

/// Minimal URL-encoder for inlining an SVG into a `data:image/svg+xml,...`
/// attribute value. Encodes only the characters that *must* be escaped:
/// `%` (encoding sentinel), `#` (URL fragment), `<`/`>`/`"` (HTML attribute
/// boundary safety). Lets us embed the domino logo as a favicon without
/// adding a base64 dependency or shipping the SVG as a separate file.
fn url_encode_svg(svg: &str) -> String {
  let mut out = String::with_capacity(svg.len() + 32);
  for c in svg.chars() {
    match c {
      '%' => out.push_str("%25"),
      '#' => out.push_str("%23"),
      '<' => out.push_str("%3C"),
      '>' => out.push_str("%3E"),
      '"' => out.push_str("%22"),
      _ => out.push(c),
    }
  }
  out
}

/// Human-readable relative time used in the hero eyebrow. Dep-free: keeps
/// the report from pulling chrono/time just to render "12 minutes ago".
/// Falls back to an absolute date for runs older than 7 days.
fn format_relative_time(unix_secs: i64) -> String {
  use std::time::{SystemTime, UNIX_EPOCH};
  let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0);
  let delta = now.saturating_sub(unix_secs);
  match delta {
    d if d < 0 => "just now".to_string(),
    d if d < 45 => "just now".to_string(),
    d if d < 90 => "a minute ago".to_string(),
    d if d < 60 * 45 => format!("{} minutes ago", d / 60),
    d if d < 60 * 90 => "an hour ago".to_string(),
    d if d < 60 * 60 * 22 => format!("{} hours ago", d / 3600),
    d if d < 60 * 60 * 36 => "yesterday".to_string(),
    d if d < 60 * 60 * 24 * 7 => format!("{} days ago", d / 86400),
    _ => {
      // >7 days: surface the absolute Unix time so the value is at least
      // self-consistent. Browsers can format this via the embedded JSON
      // metadata if a human-readable date is needed.
      format!("{} (run timestamp)", unix_secs)
    }
  }
}

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
    <title>domino · Affected Projects Report</title>
    <link rel="icon" type="image/svg+xml" href="data:image/svg+xml,{favicon}">
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@300;400;500;600;700&family=IBM+Plex+Mono:wght@400;500;600&display=swap" rel="stylesheet">
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
        const graphData = {graph};
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
                            'background-color': '#21262d',
                            'label': 'data(label)',
                            'color': '#e6edf3',
                            'text-valign': 'center',
                            'text-halign': 'center',
                            'font-size': '12px',
                            'font-weight': '500',
                            'font-family': "'IBM Plex Sans', system-ui, sans-serif",
                            'width': 'label',
                            'height': 'label',
                            'padding': '14px',
                            'shape': 'roundrectangle',
                            'text-wrap': 'wrap',
                            'text-max-width': '180px'
                        }}
                    }},
                    {{
                        selector: 'node[type="direct"]',
                        style: {{
                            'background-color': '#F26F0D',
                            'border-width': '3px',
                            'border-color': '#c25608'
                        }}
                    }},
                    {{
                        selector: 'node[type="affected"]',
                        style: {{
                            'background-color': '#79c0ff',
                            'border-width': '2px',
                            'border-color': '#388bfd',
                            'color': '#0d1117'
                        }}
                    }},
                    {{
                        selector: 'edge',
                        style: {{
                            'width': 1.5,
                            'line-color': '#30363d',
                            'target-arrow-color': '#30363d',
                            'target-arrow-shape': 'triangle',
                            'curve-style': 'bezier',
                            'label': 'data(label)',
                            'font-size': '10px',
                            'color': '#8b949e',
                            'font-family': "'IBM Plex Mono', monospace",
                            'text-background-color': '#0d1117',
                            'text-background-opacity': 0.85,
                            'text-background-padding': '3px'
                        }}
                    }},
                    {{
                        selector: 'edge[type="implicit"]',
                        style: {{
                            'line-style': 'dashed',
                            'line-color': '#e3b341',
                            'target-arrow-color': '#e3b341'
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
        :root {{
            --bg:            #0d1117;
            --bg-surface:    #161b22;
            --bg-surface-2:  #1c2128;
            --bg-elevated:   #21262d;
            --border:        #30363d;
            --border-subtle: #21262d;
            --text:          #e6edf3;
            --text-muted:    #8b949e;
            --text-subtle:   #6e7681;
            --accent:        #f26f0d;
            --accent-soft:   rgba(242, 111, 13, 0.12);
            --accent-ring:   rgba(242, 111, 13, 0.35);
            --accent-2:      #d2a8ff;
            --accent-3:      #79c0ff;
            --accent-4:      #56d364;
            --warn:          #e3b341;
            --slate:         #94a3b8;
            --font-sans:     'IBM Plex Sans', system-ui, -apple-system, sans-serif;
            --font-mono:     'IBM Plex Mono', 'JetBrains Mono', ui-monospace, monospace;
            --radius:        8px;
            --radius-sm:     4px;
            --radius-pill:   999px;
        }}

        *, *::before, *::after {{
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }}

        html {{
            scroll-behavior: smooth;
            scroll-padding-top: 72px;
        }}

        body {{
            font-family: var(--font-sans);
            background: var(--bg);
            color: var(--text);
            font-size: 14px;
            line-height: 1.6;
            font-feature-settings: "ss01", "cv11";
            -webkit-font-smoothing: antialiased;
            -moz-osx-font-smoothing: grayscale;
            min-height: 100vh;
            background-image:
                radial-gradient(circle at 20% -10%, rgba(242,111,13,0.04) 0%, transparent 40%),
                radial-gradient(circle at 80% 110%, rgba(121,192,255,0.03) 0%, transparent 50%);
            background-attachment: fixed;
        }}

        a {{
            color: var(--accent-3);
            text-decoration: none;
        }}

        a:hover {{
            text-decoration: underline;
            text-underline-offset: 3px;
        }}

        code, .mono {{
            font-family: var(--font-mono);
            font-size: 0.92em;
        }}

        /* ── Top bar ────────────────────────────────────────────────── */
        .topbar {{
            position: sticky;
            top: 0;
            z-index: 100;
            display: flex;
            align-items: center;
            gap: 20px;
            height: 56px;
            padding: 0 28px;
            background: rgba(13, 17, 23, 0.82);
            backdrop-filter: saturate(180%) blur(12px);
            -webkit-backdrop-filter: saturate(180%) blur(12px);
            border-bottom: 1px solid var(--border-subtle);
        }}

        .brand {{
            display: flex;
            align-items: center;
            gap: 10px;
            color: var(--text);
            font-weight: 600;
            font-size: 15px;
            letter-spacing: -0.01em;
        }}

        .brand:hover {{
            text-decoration: none;
            color: var(--text);
        }}

        .brand svg {{
            width: 28px;
            height: 28px;
            display: block;
        }}

        .brand-version {{
            font-family: var(--font-mono);
            font-size: 11px;
            font-weight: 500;
            color: var(--text-muted);
            background: var(--bg-elevated);
            border: 1px solid var(--border);
            padding: 2px 8px;
            border-radius: var(--radius-pill);
            letter-spacing: 0.02em;
        }}

        .topbar-eyebrow {{
            font-family: var(--font-mono);
            font-size: 11px;
            text-transform: uppercase;
            letter-spacing: 0.12em;
            color: var(--text-subtle);
            padding-left: 20px;
            border-left: 1px solid var(--border-subtle);
        }}

        .topbar-actions {{
            margin-left: auto;
            display: flex;
            align-items: center;
            gap: 12px;
        }}

        .topbar-link {{
            display: inline-flex;
            align-items: center;
            gap: 8px;
            color: var(--text-muted);
            font-size: 13px;
            font-weight: 500;
            padding: 6px 12px;
            border-radius: var(--radius-sm);
            transition: color 120ms ease, background 120ms ease;
        }}

        .topbar-link:hover {{
            color: var(--text);
            background: var(--bg-elevated);
            text-decoration: none;
        }}

        .topbar-link svg:first-child {{
            opacity: 0.85;
        }}

        .topbar-link svg:last-child {{
            opacity: 0.6;
        }}

        /* ── Main container ────────────────────────────────────────── */
        .container {{
            max-width: 1240px;
            margin: 0 auto;
            padding: 56px 28px 80px;
        }}

        /* ── Hero ──────────────────────────────────────────────────── */
        .hero {{
            margin-bottom: 56px;
        }}

        .hero-eyebrow {{
            font-family: var(--font-mono);
            font-size: 11px;
            text-transform: uppercase;
            letter-spacing: 0.16em;
            color: var(--text-subtle);
            margin-bottom: 20px;
            display: flex;
            align-items: center;
            gap: 14px;
        }}

        .hero-eyebrow::before {{
            content: '';
            display: inline-block;
            width: 24px;
            height: 1px;
            background: var(--accent);
        }}

        .hero-title {{
            display: flex;
            align-items: baseline;
            gap: 20px;
            flex-wrap: wrap;
            font-weight: 400;
            letter-spacing: -0.02em;
            margin-bottom: 28px;
        }}

        .hero-count {{
            font-family: var(--font-mono);
            font-size: clamp(48px, 7vw, 84px);
            font-weight: 600;
            font-variant-numeric: tabular-nums;
            line-height: 1;
            color: var(--text);
        }}

        .hero-label {{
            font-size: clamp(22px, 2.4vw, 30px);
            font-weight: 300;
            color: var(--text-muted);
            line-height: 1.2;
        }}

        /* ── Global-invalidation banner ────────────────────────────── */
        .global-banner {{
            position: relative;
            background: var(--bg-surface);
            border: 1px solid var(--border);
            border-radius: var(--radius);
            padding: 24px 28px 24px 32px;
            margin-bottom: 40px;
            overflow: hidden;
        }}

        .global-banner::before {{
            content: '';
            position: absolute;
            left: 0;
            top: 0;
            bottom: 0;
            width: 3px;
            background: var(--accent);
        }}

        .global-banner-head {{
            display: flex;
            align-items: center;
            gap: 12px;
            margin-bottom: 12px;
        }}

        .global-banner-head svg {{
            color: var(--accent);
            flex-shrink: 0;
        }}

        .global-banner h3 {{
            font-size: 16px;
            font-weight: 600;
            color: var(--text);
            margin: 0;
            letter-spacing: -0.005em;
        }}

        .global-banner p {{
            font-size: 13.5px;
            color: var(--text-muted);
            line-height: 1.65;
            margin: 8px 0;
            max-width: 78ch;
        }}

        .global-banner p strong {{
            color: var(--text);
            font-weight: 600;
        }}

        .global-banner-list {{
            margin: 16px 0 14px;
            display: flex;
            flex-direction: column;
            gap: 6px;
        }}

        .global-banner-row {{
            display: grid;
            grid-template-columns: minmax(200px, 1fr) auto minmax(140px, 1fr);
            align-items: center;
            gap: 16px;
            padding: 8px 14px;
            background: var(--bg);
            border: 1px solid var(--border-subtle);
            border-radius: var(--radius-sm);
            font-family: var(--font-mono);
            font-size: 12.5px;
        }}

        .global-banner-row .file {{
            color: var(--text);
            font-weight: 500;
        }}

        .global-banner-row .arrow {{
            color: var(--text-subtle);
            font-size: 11px;
        }}

        .global-banner-row .input {{
            color: var(--accent);
            font-weight: 500;
        }}

        .global-banner-row .pattern {{
            grid-column: 1 / -1;
            font-size: 11px;
            color: var(--text-subtle);
            border-top: 1px dashed var(--border-subtle);
            padding-top: 6px;
            margin-top: 2px;
        }}

        .global-banner .docs-link {{
            display: inline-flex;
            align-items: center;
            gap: 6px;
            color: var(--accent-3);
            font-size: 13px;
            font-weight: 500;
            margin-top: 4px;
        }}

        /* ── Stats grid ────────────────────────────────────────────── */
        .stats-grid {{
            display: grid;
            grid-template-columns: repeat(4, minmax(0, 1fr));
            gap: 12px;
            margin-bottom: 32px;
        }}

        .stat-card {{
            background: var(--bg-surface);
            border: 1px solid var(--border-subtle);
            border-radius: var(--radius);
            padding: 22px 22px 20px;
            position: relative;
            overflow: hidden;
        }}

        .stat-card-accent::before {{
            content: '';
            position: absolute;
            top: 0;
            left: 22px;
            width: 18px;
            height: 2px;
            background: var(--accent);
        }}

        .stat-value {{
            font-family: var(--font-mono);
            font-size: 36px;
            font-weight: 500;
            font-variant-numeric: tabular-nums;
            color: var(--text);
            letter-spacing: -0.02em;
            line-height: 1.1;
            margin-bottom: 8px;
        }}

        .stat-label {{
            font-size: 11px;
            font-weight: 500;
            text-transform: uppercase;
            letter-spacing: 0.1em;
            color: var(--text-subtle);
        }}

        @media (max-width: 980px) {{
            .stats-grid {{
                grid-template-columns: repeat(2, minmax(0, 1fr));
            }}
        }}

        @media (max-width: 520px) {{
            .stats-grid {{
                grid-template-columns: 1fr;
            }}
        }}

        /* ── Card primitive ────────────────────────────────────────── */
        .card {{
            background: var(--bg-surface);
            border: 1px solid var(--border-subtle);
            border-radius: var(--radius);
            margin-bottom: 32px;
            overflow: hidden;
        }}

        .card-header {{
            display: flex;
            align-items: baseline;
            justify-content: space-between;
            gap: 16px;
            padding: 20px 24px 4px;
        }}

        .card-title {{
            font-size: 15px;
            font-weight: 600;
            color: var(--text);
            letter-spacing: -0.005em;
        }}

        .card-sub {{
            font-size: 12.5px;
            color: var(--text-subtle);
            padding: 0 24px;
            margin-bottom: 16px;
        }}

        .card-body {{
            padding: 0 24px 24px;
        }}

        /* ── Graph card ────────────────────────────────────────────── */
        .graph-toolbar {{
            display: flex;
            align-items: center;
            flex-wrap: wrap;
            gap: 14px;
            padding: 12px 24px;
            background: var(--bg);
            border-top: 1px solid var(--border-subtle);
            border-bottom: 1px solid var(--border-subtle);
        }}

        .toolbar-label {{
            font-family: var(--font-mono);
            font-size: 11px;
            text-transform: uppercase;
            letter-spacing: 0.1em;
            color: var(--text-subtle);
        }}

        .layout-buttons {{
            display: flex;
            flex-wrap: wrap;
            gap: 4px;
        }}

        .layout-btn {{
            font-family: var(--font-mono);
            font-size: 12px;
            font-weight: 500;
            color: var(--text-muted);
            background: transparent;
            border: 1px solid transparent;
            padding: 5px 11px;
            border-radius: var(--radius-sm);
            cursor: pointer;
            transition: color 120ms, background 120ms, border-color 120ms;
        }}

        .layout-btn:hover {{
            color: var(--text);
            background: var(--bg-elevated);
        }}

        .layout-btn.active {{
            color: var(--text);
            background: var(--accent-soft);
            border-color: var(--accent-ring);
        }}

        .graph-legend {{
            display: flex;
            flex-wrap: wrap;
            gap: 18px;
            padding: 14px 24px;
            background: var(--bg);
            border-bottom: 1px solid var(--border-subtle);
            font-size: 12px;
            color: var(--text-muted);
        }}

        .legend-item {{
            display: inline-flex;
            align-items: center;
            gap: 8px;
        }}

        .legend-dot {{
            width: 10px;
            height: 10px;
            border-radius: 2px;
        }}

        .legend-dot.direct  {{ background: var(--accent); }}
        .legend-dot.affected{{ background: var(--accent-3); }}

        .legend-line {{
            width: 22px;
            height: 0;
            border-top: 1.5px solid var(--text-subtle);
        }}

        .legend-line.implicit {{
            border-top-style: dashed;
            border-top-color: var(--warn);
        }}

        #cy {{
            width: 100%;
            height: 540px;
            background: var(--bg);
        }}

        /* ── Filter bar ────────────────────────────────────────────── */
        .filter-controls {{
            display: flex;
            align-items: center;
            gap: 4px;
            padding: 12px 24px;
            background: var(--bg);
            border-top: 1px solid var(--border-subtle);
            border-bottom: 1px solid var(--border-subtle);
            flex-wrap: wrap;
        }}

        .filter-btn {{
            font-family: var(--font-mono);
            font-size: 12px;
            font-weight: 500;
            color: var(--text-muted);
            background: transparent;
            border: 1px solid transparent;
            padding: 5px 12px;
            border-radius: var(--radius-pill);
            cursor: pointer;
            transition: color 120ms, background 120ms, border-color 120ms;
        }}

        .filter-btn:hover {{
            color: var(--text);
            background: var(--bg-elevated);
        }}

        .filter-btn.active {{
            color: var(--text);
            background: var(--accent-soft);
            border-color: var(--accent-ring);
        }}

        .ghost-btn {{
            font-family: var(--font-mono);
            font-size: 12px;
            color: var(--text-muted);
            background: transparent;
            border: 1px solid var(--border);
            padding: 5px 12px;
            border-radius: var(--radius-sm);
            cursor: pointer;
            transition: color 120ms, background 120ms;
        }}

        .ghost-btn:hover {{
            color: var(--text);
            background: var(--bg-elevated);
        }}

        /* ── Project cards ─────────────────────────────────────────── */
        .details-body {{
            padding: 8px 24px 24px;
            display: flex;
            flex-direction: column;
            gap: 10px;
        }}

        .project-card {{
            background: var(--bg);
            border: 1px solid var(--border-subtle);
            border-radius: var(--radius);
            transition: border-color 120ms ease;
        }}

        .project-card:hover {{
            border-color: var(--border);
        }}

        .project-card.hidden {{
            display: none;
        }}

        .project-card > details > summary {{
            cursor: pointer;
            list-style: none;
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 16px;
            padding: 14px 18px;
            user-select: none;
        }}

        .project-card > details > summary::-webkit-details-marker {{
            display: none;
        }}

        .project-card > details > summary::before {{
            content: '';
            display: inline-block;
            width: 0;
            height: 0;
            border-left: 5px solid var(--text-subtle);
            border-top: 4px solid transparent;
            border-bottom: 4px solid transparent;
            transition: transform 120ms;
            flex-shrink: 0;
        }}

        .project-card > details[open] > summary::before {{
            transform: rotate(90deg);
        }}

        .project-name {{
            display: inline-flex;
            align-items: center;
            gap: 10px;
            font-size: 14px;
            font-weight: 500;
            color: var(--text);
            flex: 1;
            min-width: 0;
        }}

        .project-name svg {{
            color: var(--text-subtle);
            flex-shrink: 0;
        }}

        .badge-container {{
            display: inline-flex;
            align-items: center;
            gap: 8px;
            flex-shrink: 0;
        }}

        .affect-badge {{
            font-family: var(--font-mono);
            font-size: 10.5px;
            font-weight: 500;
            text-transform: uppercase;
            letter-spacing: 0.06em;
            padding: 3px 9px;
            border-radius: var(--radius-pill);
            border: 1px solid var(--border);
            color: var(--text-muted);
            background: var(--bg-elevated);
        }}

        .affect-badge.badge-direct {{
            color: var(--accent);
            border-color: var(--accent-ring);
            background: var(--accent-soft);
        }}

        .affect-badge.badge-affected {{
            color: var(--accent-3);
            border-color: rgba(121, 192, 255, 0.3);
            background: rgba(121, 192, 255, 0.08);
        }}

        .affect-badge.badge-both {{
            color: var(--text);
            border-color: var(--border);
            background: var(--bg-elevated);
        }}

        .cause-list-container {{
            padding: 4px 18px 18px 18px;
        }}

        .cause-list {{
            list-style: none;
            display: flex;
            flex-direction: column;
            gap: 8px;
        }}

        .cause-item {{
            background: var(--bg-surface);
            border: 1px solid var(--border-subtle);
            border-left: 2px solid var(--border);
            border-radius: var(--radius-sm);
            padding: 12px 14px;
        }}

        /* Each cause-item gets its left edge tinted to match the pill,
           so a glance reveals the cause type without reading the pill. */
        .cause-item:has(.cause-type.direct)   {{ border-left-color: var(--accent); }}
        .cause-item:has(.cause-type.imported) {{ border-left-color: var(--accent-3); }}
        .cause-item:has(.cause-type.reexported){{ border-left-color: var(--accent-2); }}
        .cause-item:has(.cause-type.implicit) {{ border-left-color: var(--accent-4); }}
        .cause-item:has(.cause-type.asset)    {{ border-left-color: var(--accent-3); }}
        .cause-item:has(.cause-type.lockfile) {{ border-left-color: var(--warn); }}
        .cause-item:has(.cause-type.global)   {{ border-left-color: var(--slate); }}

        .cause-type {{
            display: inline-block;
            font-family: var(--font-mono);
            font-size: 10.5px;
            font-weight: 500;
            text-transform: uppercase;
            letter-spacing: 0.08em;
            padding: 3px 9px;
            border-radius: var(--radius-pill);
            margin-bottom: 8px;
            color: var(--text-muted);
            background: var(--bg-elevated);
            border: 1px solid var(--border);
        }}

        .cause-type.direct {{
            color: var(--accent);
            background: var(--accent-soft);
            border-color: var(--accent-ring);
        }}

        .cause-type.imported {{
            color: var(--accent-3);
            background: rgba(121, 192, 255, 0.10);
            border-color: rgba(121, 192, 255, 0.30);
        }}

        .cause-type.reexported {{
            color: var(--accent-2);
            background: rgba(210, 168, 255, 0.10);
            border-color: rgba(210, 168, 255, 0.30);
        }}

        .cause-type.implicit {{
            color: var(--accent-4);
            background: rgba(86, 211, 100, 0.10);
            border-color: rgba(86, 211, 100, 0.30);
        }}

        .cause-type.asset {{
            color: #56d3c5;
            background: rgba(86, 211, 197, 0.10);
            border-color: rgba(86, 211, 197, 0.28);
        }}

        .cause-type.lockfile {{
            color: var(--warn);
            background: rgba(227, 179, 65, 0.10);
            border-color: rgba(227, 179, 65, 0.30);
        }}

        .cause-type.global {{
            color: var(--slate);
            background: rgba(148, 163, 184, 0.10);
            border-color: rgba(148, 163, 184, 0.30);
        }}

        .cause-details {{
            color: var(--text-muted);
            font-size: 13px;
            line-height: 1.6;
        }}

        .code-path {{
            font-family: var(--font-mono);
            background: var(--bg);
            padding: 1px 6px;
            border-radius: 3px;
            color: var(--accent);
            font-size: 0.92em;
            border: 1px solid var(--border-subtle);
        }}

        .symbol {{
            font-family: var(--font-mono);
            background: var(--bg);
            padding: 1px 6px;
            border-radius: 3px;
            color: var(--accent-2);
            font-size: 0.92em;
            border: 1px solid var(--border-subtle);
        }}

        .cause-details code {{
            font-family: var(--font-mono);
            color: var(--text);
            background: var(--bg);
            padding: 1px 6px;
            border-radius: 3px;
            font-size: 0.9em;
            border: 1px solid var(--border-subtle);
        }}

        /* ── Collapsed global-only group ───────────────────────────── */
        .global-only-group {{
            margin-top: 24px;
            background: var(--bg);
            border: 1px solid var(--border-subtle);
            border-left: 2px solid var(--slate);
            border-radius: var(--radius);
            padding: 14px 18px;
        }}

        .global-only-group > summary {{
            cursor: pointer;
            list-style: none;
            color: var(--text-muted);
            font-size: 13px;
            font-family: var(--font-mono);
            user-select: none;
            display: flex;
            align-items: center;
            gap: 10px;
        }}

        .global-only-group > summary::-webkit-details-marker {{
            display: none;
        }}

        .global-only-group > summary::before {{
            content: '';
            display: inline-block;
            width: 0;
            height: 0;
            border-left: 5px solid var(--text-subtle);
            border-top: 4px solid transparent;
            border-bottom: 4px solid transparent;
            transition: transform 120ms;
            flex-shrink: 0;
        }}

        .global-only-group[open] > summary::before {{
            transform: rotate(90deg);
        }}

        .global-only-group > summary strong {{
            color: var(--text);
            font-weight: 500;
        }}

        .global-only-inner {{
            margin-top: 12px;
            display: flex;
            flex-direction: column;
            gap: 8px;
        }}

        /* ── Footer ────────────────────────────────────────────────── */
        .footer {{
            border-top: 1px solid var(--border-subtle);
            padding: 32px 28px 48px;
            margin-top: 24px;
        }}

        .footer-grid {{
            max-width: 1240px;
            margin: 0 auto;
            display: grid;
            grid-template-columns: 1.4fr 1fr 1fr;
            gap: 32px;
            align-items: start;
        }}

        @media (max-width: 720px) {{
            .footer-grid {{
                grid-template-columns: 1fr;
                gap: 20px;
            }}
        }}

        .footer-brand {{
            display: flex;
            flex-direction: column;
            gap: 6px;
        }}

        .footer-brand-row {{
            display: inline-flex;
            align-items: center;
            gap: 10px;
            color: var(--text);
            font-weight: 600;
            font-size: 14px;
        }}

        .footer-brand-row svg {{
            width: 22px;
            height: 22px;
        }}

        .footer-sub {{
            font-size: 12.5px;
            color: var(--text-subtle);
            max-width: 32ch;
        }}

        .footer-col-title {{
            font-family: var(--font-mono);
            font-size: 10.5px;
            text-transform: uppercase;
            letter-spacing: 0.12em;
            color: var(--text-subtle);
            margin-bottom: 8px;
        }}

        .footer-col p {{
            font-size: 13px;
            color: var(--text-muted);
            line-height: 1.7;
            max-width: 36ch;
        }}

        .footer-col p code {{
            /* Keep code chips on a single line so `nx affected` / `#domino-meta`
               don't fragment in the middle of a name. */
            white-space: nowrap;
        }}

        .footer-col a {{
            font-size: 13px;
            color: var(--text-muted);
            display: inline-flex;
            align-items: center;
            gap: 6px;
        }}

        .footer-col code {{
            font-family: var(--font-mono);
            color: var(--accent);
        }}

        .footer-links {{
            display: flex;
            flex-direction: column;
            gap: 8px;
        }}

        /* ── Motion ────────────────────────────────────────────────── */
        @media (prefers-reduced-motion: no-preference) {{
            .hero, .global-banner, .stats-grid, .card {{
                animation: domino-fade-up 320ms cubic-bezier(0.16, 1, 0.3, 1) backwards;
            }}
            .hero          {{ animation-delay: 0ms; }}
            .global-banner {{ animation-delay: 40ms; }}
            .stats-grid    {{ animation-delay: 80ms; }}
            .card          {{ animation-delay: 120ms; }}

            @keyframes domino-fade-up {{
                from {{ opacity: 0; transform: translateY(8px); }}
                to   {{ opacity: 1; transform: translateY(0);   }}
            }}
        }}

        /* ── Focus visibility ──────────────────────────────────────── */
        :focus-visible {{
            outline: 2px solid var(--accent);
            outline-offset: 2px;
            border-radius: var(--radius-sm);
        }}

        /* ── Print ─────────────────────────────────────────────────── */
        @media print {{
            .topbar, .graph-toolbar, .filter-controls, .ghost-btn {{ display: none; }}
            body {{ background: #fff; color: #000; }}
            .card, .stat-card, .project-card, .global-banner {{
                background: #fff;
                border-color: #ddd;
                page-break-inside: avoid;
            }}
        }}
    </style>
</head>
<body>
    {metadata}
    <header class="topbar">
        <a class="brand" href="{repo}" target="_blank" rel="noopener noreferrer" aria-label="domino on GitHub">
            {logo}
            <span>domino</span>
            <span class="brand-version">v{version}</span>
        </a>
        <span class="topbar-eyebrow">Affected Projects Report</span>
        <div class="topbar-actions">
            <a class="topbar-link" href="{repo}" target="_blank" rel="noopener noreferrer">
                {icon_github}
                <span>Star on GitHub</span>
                {icon_external}
            </a>
        </div>
    </header>

    <main class="container">
        <section class="hero" aria-labelledby="hero-title">
            <div class="hero-eyebrow">Generated {eyebrow} &middot; v{version}</div>
            <h1 class="hero-title" id="hero-title">
                <span class="hero-count">{count}</span>
                <span class="hero-label">project{plural} affected</span>
            </h1>
        </section>

        <section class="stats-grid" aria-label="Summary statistics">
            <div class="stat-card">
                <div class="stat-value">{total_causes}</div>
                <div class="stat-label">Total Causes</div>
            </div>
            <div class="stat-card stat-card-accent" style="{split_style}">
                <div class="stat-value">{globally_invalidated}</div>
                <div class="stat-label">Globally Invalidated</div>
            </div>
            <div class="stat-card" style="{split_style}">
                <div class="stat-value">{semantically_affected}</div>
                <div class="stat-label">Semantically Affected</div>
            </div>
            <div class="stat-card">
                <div class="stat-value">{changed_files}</div>
                <div class="stat-label">Changed Files</div>
            </div>
        </section>

        {banner}

        <section class="card graph-card">
            <header class="card-header">
                <h2 class="card-title">Dependency graph</h2>
            </header>
            <p class="card-sub">Pan, zoom, and drag nodes to explore &mdash; hover for details.</p>
            <div class="graph-legend">
                <span class="legend-item"><span class="legend-dot direct"></span>Direct change</span>
                <span class="legend-item"><span class="legend-dot affected"></span>Affected project</span>
                <span class="legend-item"><span class="legend-line"></span>Import dependency</span>
                <span class="legend-item"><span class="legend-line implicit"></span>Implicit dependency</span>
            </div>
            <div class="graph-toolbar">
                <span class="toolbar-label">Layout</span>
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
        </section>

        <section class="card details-card">
            <header class="card-header">
                <h2 class="card-title">Detailed impact analysis</h2>
                <button id="toggleAllBtn" class="ghost-btn" onclick="toggleAllDetails()">Expand all</button>
            </header>
            <div class="filter-controls">
                <button class="filter-btn active" onclick="filterProjects('all'); setActiveFilter(this)">All</button>
                <button class="filter-btn" onclick="filterProjects('direct'); setActiveFilter(this)">Direct</button>
                <button class="filter-btn" onclick="filterProjects('affected'); setActiveFilter(this)">Affected</button>
                <button class="filter-btn" onclick="filterProjects('both'); setActiveFilter(this)">Both</button>
            </div>
            <div class="details-body">
                {details}
            </div>
        </section>
    </main>

    <footer class="footer">
        <div class="footer-grid">
            <div class="footer-col footer-brand">
                <div class="footer-brand-row">
                    {logo_small}
                    <span>domino &middot; v{version}</span>
                </div>
                <p class="footer-sub">Semantic change detection for monorepos &mdash; a drop-in <code>nx affected</code> replacement built on the Oxc parser.</p>
            </div>
            <div class="footer-col">
                <div class="footer-col-title">Machine-readable</div>
                <p>Run metadata is embedded as JSON at <code>#domino-meta</code> in this document.</p>
            </div>
            <div class="footer-col footer-links">
                <div class="footer-col-title">Project</div>
                <a href="{repo}" target="_blank" rel="noopener noreferrer">{icon_github_sm} GitHub</a>
                <a href="{repo}/issues" target="_blank" rel="noopener noreferrer">Report an issue {icon_external}</a>
                <a href="{repo}/blob/main/LICENSE" target="_blank" rel="noopener noreferrer">MIT License {icon_external}</a>
            </div>
        </div>
    </footer>
</body>
</html>"#,
    graph = graph_data,
    metadata = metadata_script,
    banner = banner_html,
    logo = LOGO_SVG,
    logo_small = LOGO_SVG,
    favicon = url_encode_svg(LOGO_SVG),
    repo = REPO_URL,
    icon_github = ICON_GITHUB,
    icon_github_sm = ICON_GITHUB,
    icon_external = ICON_EXTERNAL,
    eyebrow = format_relative_time(report.run_started_at_unix_secs),
    version = env!("CARGO_PKG_VERSION"),
    count = format_number(report.projects.len()),
    plural = if report.projects.len() == 1 { "" } else { "s" },
    total_causes = format_number(total_causes),
    changed_files = format_number(report.totals.changed_files),
    split_style = summary_split_style,
    globally_invalidated = format_number(report.totals.globally_invalidated),
    semantically_affected = format_number(report.totals.semantically_affected),
    details = details_html,
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
  let mut node_ids: HashSet<String> = HashSet::new();
  for project in &report.projects {
    let node_type = if direct_changes.contains(&project.name) {
      "direct"
    } else {
      "affected"
    };

    let node_id = sanitize_node_id(&project.name);
    nodes.push(format!(
      r#"{{ data: {{ id: "{}", label: "{}", type: "{}" }} }}"#,
      node_id, project.name, node_type
    ));
    node_ids.insert(node_id);
  }

  // Build edges array. Skip any edge whose source or target isn't in the
  // node set — cytoscape throws on dangling edges, and a cause may reference
  // a project that isn't itself on the affected list (e.g. an upstream lib
  // that the affected detector pruned because nothing else points to it).
  let mut edges = Vec::new();
  for (source, targets) in relationships {
    let source_id = sanitize_node_id(&source);
    if !node_ids.contains(&source_id) {
      continue;
    }
    for (target, cause_types) in targets {
      let target_id = sanitize_node_id(&target);
      if !node_ids.contains(&target_id) {
        continue;
      }

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

  let mut rows = String::new();
  for trigger in &report.global_triggers {
    rows.push_str(&format!(
      r#"<div class="global-banner-row">
                <span class="file">{file}</span>
                <span class="arrow">&larr; matched</span>
                <span class="input">{input}</span>
                <span class="pattern">{pattern}</span>
            </div>"#,
      file = html_escape(&trigger.file.display().to_string()),
      input = html_escape(&trigger.named_input),
      pattern = html_escape(&trigger.raw_pattern),
    ));
  }

  format!(
    r#"<section class="global-banner" role="status" aria-labelledby="gbanner-title">
            <div class="global-banner-head">
                {icon}
                <h3 id="gbanner-title">Global invalidation detected</h3>
            </div>
            <p>
                <strong>{trigger_count}</strong> changed file{trigger_plural} matched Nx <code>namedInputs</code>
                workspace-root pattern{trigger_plural}, so all <strong>{project_count}</strong> projects
                are marked affected &mdash; the same as <code>nx affected</code> would do.
            </p>
            <div class="global-banner-list">{rows}</div>
            <p>
                Semantic analysis of the other changed files still ran &mdash; the
                <em>Detailed impact analysis</em> section below separates projects affected by
                real code signal from those only swept up by the global rule.
            </p>
            <a class="docs-link" href="https://nx.dev/concepts/more-concepts/customizing-inputs" target="_blank" rel="noopener noreferrer">
                Learn how Nx <code>namedInputs</code> work {external}
            </a>
        </section>"#,
    icon = ICON_INFO,
    external = ICON_EXTERNAL,
    trigger_count = trigger_count,
    trigger_plural = if trigger_count == 1 { "" } else { "s" },
    project_count = format_number(project_count),
    rows = rows,
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
        // Real direct-file signals. GlobalInvalidation is deliberately
        // excluded here — a workspace-rule sweep isn't the same kind of
        // "direct change" as a real code edit, and conflating them is
        // exactly the UX bug the global-invalidation work was about.
        AffectCause::DirectChange { .. }
        | AffectCause::AssetChange { .. }
        | AffectCause::LockfileChange { .. } => has_direct = true,
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
        r#"<span class="affect-badge badge-direct">Direct</span>"#,
        "direct",
      )
    } else {
      (
        r#"<span class="affect-badge badge-affected">Affected</span>"#,
        "affected",
      )
    };

    html.push_str(&format!(
      r#"<div class="project-card" data-filter-type="{filter}">
                <details>
                    <summary>
                        <div class="project-name">{icon}<span>{name}</span></div>
                        <div class="badge-container">
                            {badge}
                            <span class="affect-badge">{count} cause{plural}</span>
                        </div>
                    </summary>
                    <div class="cause-list-container">
                        <ul class="cause-list">
"#,
      filter = filter_type,
      icon = ICON_PACKAGE,
      name = html_escape(&project.name),
      badge = badge,
      count = project.causes.len(),
      plural = if project.causes.len() == 1 { "" } else { "s" }
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
          html.push_str("<span class=\"cause-type asset\">Asset Change</span>");
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
          html.push_str("<span class=\"cause-type lockfile\">Lockfile Change</span>");
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
                <summary><strong>{count}</strong> projects globally invalidated{label}</summary>
                <div class="global-only-inner">"#,
      count = format_number(global_only.len()),
      label = group_label,
    ));

    for project in &global_only {
      html.push_str(&format!(
        r#"<div class="project-card" data-filter-type="affected">
                    <details>
                        <summary>
                            <div class="project-name">{icon}<span>{name}</span></div>
                            <div class="badge-container">
                                <span class="affect-badge">Global only</span>
                                <span class="affect-badge">{count} cause{plural}</span>
                            </div>
                        </summary>
                        <div class="cause-list-container">
                            <ul class="cause-list">"#,
        icon = ICON_PACKAGE,
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

  /// Unix seconds for "12 minutes ago" — keeps the sample report's eyebrow
  /// showing a friendly relative time instead of the >7-day fallback.
  fn synth_recent_timestamp() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_secs() as i64)
      .unwrap_or(0);
    now - 12 * 60
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
    // Two upstream libs that the 15 apps import from. Including them as
    // real projects (with their own causes) means the dependency graph has
    // valid edge endpoints — the apps point to `shared-utils`, which in
    // turn imports from `core`. Without these the synth graph has dangling
    // edges and cytoscape (rightly) refuses to render.
    projects.push(make_project(
      "shared-utils",
      vec![
        AffectCause::DirectChange {
          file: PathBuf::from("libs/shared-utils/src/format.ts"),
          symbol: Some("formatDate".to_string()),
          line: 18,
        },
        AffectCause::ImportedSymbol {
          source_project: "core".to_string(),
          symbol: "Logger".to_string(),
          via_file: PathBuf::from("libs/shared-utils/src/log.ts"),
          source_file: PathBuf::from("libs/core/src/logger.ts"),
        },
        AffectCause::GlobalInvalidation {
          file: PathBuf::from(".github/workflows/ci.yml"),
          named_input: "sharedGlobals".to_string(),
        },
      ],
    ));
    projects.push(make_project(
      "core",
      vec![
        AffectCause::DirectChange {
          file: PathBuf::from("libs/core/src/logger.ts"),
          symbol: Some("Logger".to_string()),
          line: 42,
        },
        AffectCause::GlobalInvalidation {
          file: PathBuf::from(".github/workflows/ci.yml"),
          named_input: "sharedGlobals".to_string(),
        },
      ],
    ));
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
        semantically_affected: 17,
        overlap: 17,
        changed_files: 4,
      },
      version: env!("CARGO_PKG_VERSION"),
      // Recent timestamp so the eyebrow renders a friendly relative time
      // instead of the >7-day fallback. Computed at runtime in the harness.
      run_started_at_unix_secs: synth_recent_timestamp(),
    }
  }

  fn synth_normal_report() -> AffectedReport {
    let projects = vec![
      make_project(
        "core",
        vec![AffectCause::DirectChange {
          file: PathBuf::from("libs/core/src/logger.ts"),
          symbol: Some("Logger".to_string()),
          line: 42,
        }],
      ),
      make_project(
        "app-web",
        vec![AffectCause::ImportedSymbol {
          source_project: "shared-utils".to_string(),
          symbol: "formatDate".to_string(),
          via_file: PathBuf::from("apps/web/src/main.ts"),
          source_file: PathBuf::from("libs/shared-utils/src/format.ts"),
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
        semantically_affected: 4,
        overlap: 0,
        changed_files: 2,
      },
      version: env!("CARGO_PKG_VERSION"),
      // Recent timestamp so the eyebrow renders a friendly relative time
      // instead of the >7-day fallback. Computed at runtime in the harness.
      run_started_at_unix_secs: synth_recent_timestamp(),
    }
  }

  #[test]
  fn banner_renders_only_when_global_triggers_present() {
    let global = generate_html(&synth_global_report());
    let normal = generate_html(&synth_normal_report());

    assert!(global.contains("Global invalidation detected"));
    assert!(global.contains("cause-type global"));
    assert!(global.contains(r#"<section class="global-banner""#));
    assert!(global.contains(r#"id="domino-meta""#));

    // Non-global runs MUST remain visually identical when no global rule
    // fired — no banner element, no new pill, no collapsed group element.
    // (The CSS *classes* are still emitted in the always-included <style>
    // block; we assert on the rendered elements, not the stylesheet.)
    assert!(!normal.contains("Global invalidation detected"));
    assert!(!normal.contains("\"cause-type global\""));
    assert!(!normal.contains(r#"<section class="global-banner""#));
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

  #[test]
  fn favicon_is_embedded_inline() {
    // The report is a single-file artifact (saved, mailed, archived) — the
    // favicon must be self-contained, not a relative URL that 404s.
    let html = generate_html(&synth_normal_report());
    assert!(
      html.contains(r#"<link rel="icon" type="image/svg+xml" href="data:image/svg+xml,"#),
      "favicon must be an inline SVG data URI"
    );
    // `#` is the URI fragment separator; if any unencoded `#` slipped
    // through the helper, the browser would truncate the data URI at the
    // first color hex (e.g. #F26F0D in the logo) and the favicon would
    // render as a broken image. Guard against that regression.
    assert!(
      !html.contains(r#"href="data:image/svg+xml,<svg"#),
      "favicon data URI must url-encode `<`, otherwise some browsers/validators reject it"
    );
  }

  #[test]
  fn url_encode_svg_escapes_hash() {
    let svg = r##"<svg fill="#F26F0D"/>"##;
    let encoded = url_encode_svg(svg);
    assert!(!encoded.contains('#'), "raw # must be percent-encoded");
    assert!(encoded.contains("%23F26F0D"));
    assert!(!encoded.contains('<'));
    assert!(encoded.contains("%3Csvg"));
  }

  #[test]
  fn top_bar_renders_logo_and_github_link() {
    // Every report must carry brand identity + a path back to the project.
    let html = generate_html(&synth_normal_report());
    assert!(
      html.contains(r#"id="domino-logo""#),
      "inline domino logo must be present in the top bar"
    );
    assert!(
      html.contains("https://github.com/frontops-dev/domino"),
      "top bar must link back to the GitHub repository"
    );
    assert!(html.contains(r#"<header class="topbar""#));
    assert!(
      html.contains("Star on GitHub"),
      "GitHub link must carry an accessible label"
    );
  }

  #[test]
  fn footer_links_to_repo_issues_and_license() {
    let html = generate_html(&synth_normal_report());
    assert!(html.contains("https://github.com/frontops-dev/domino/issues"));
    assert!(html.contains("https://github.com/frontops-dev/domino/blob/main/LICENSE"));
  }

  #[test]
  fn direct_change_carries_brand_accent_pill() {
    // Encodes intent: the orange brand accent is scarce — only DirectChange
    // (the strongest signal) wears it. Other cause variants get the desaturated
    // semantic palette.
    let html = generate_html(&synth_normal_report());
    let direct = html
      .find("Direct Change")
      .expect("expected a Direct Change pill");
    let nearby = &html[direct.saturating_sub(80)..direct];
    assert!(
      nearby.contains("cause-type direct"),
      "Direct Change must use the orange-accent .direct pill class, got: {}",
      nearby
    );
    // ImportedSymbol must NOT borrow the direct class.
    let imported = html
      .find("Imported Symbol")
      .expect("expected an Imported Symbol pill");
    let near_imported = &html[imported.saturating_sub(80)..imported];
    assert!(!near_imported.contains("cause-type direct"));
  }

  #[test]
  fn relative_time_helper_buckets() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap()
      .as_secs() as i64;

    assert_eq!(format_relative_time(now), "just now");
    assert_eq!(format_relative_time(now - 30), "just now");
    assert_eq!(format_relative_time(now - 60 * 5), "5 minutes ago");
    assert_eq!(format_relative_time(now - 60 * 60 * 3), "3 hours ago");
    assert_eq!(format_relative_time(now - 60 * 60 * 26), "yesterday");
    assert_eq!(format_relative_time(now - 60 * 60 * 24 * 3), "3 days ago");
    // >7 days falls back to a raw timestamp form — assert that we don't lie
    // by emitting a relative-time string with the wrong magnitude.
    let old = format_relative_time(now - 60 * 60 * 24 * 30);
    assert!(
      old.contains("(run timestamp)"),
      "expected fallback form for >7d-old runs, got: {}",
      old
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
