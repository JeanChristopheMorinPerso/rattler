use std::{collections::BTreeSet, ops::Bound};

use miette::IntoDiagnostic;
use rattler_conda_types::{Version, VersionSpec};
use version_ranges::Ranges;

/// Debug version spec ranges.
#[derive(Debug, clap::Parser)]
pub struct Opt {
    /// The version spec to parse, e.g. ">=2,<4"
    spec: String,

    /// An optional version or version spec to check against the spec.
    /// Bare values like "1.2.3" are treated as versions; use "==1.2.3"
    /// to compare as an exact spec instead.
    against: Option<String>,
}

/// Format a set of ranges using mathematical interval notation.
fn format_ranges(ranges: &Ranges<Version>) -> String {
    let segments: Vec<String> = ranges
        .iter()
        .map(|(lo, hi)| {
            let left = match lo {
                Bound::Unbounded => "(-∞".to_string(),
                Bound::Included(v) => format!("[{v}"),
                Bound::Excluded(v) => format!("({v}"),
            };
            let right = match hi {
                Bound::Unbounded => "+∞)".to_string(),
                Bound::Included(v) => format!("{v}]"),
                Bound::Excluded(v) => format!("{v})"),
            };
            format!("{left}, {right}")
        })
        .collect();

    if segments.is_empty() {
        "∅".to_string()
    } else {
        segments.join(" ∪ ")
    }
}

fn print_version_check(spec: &VersionSpec, ranges: &Ranges<Version>, version: &Version) {
    let spec_match = spec.matches(version);
    let range_match = ranges.contains(version);
    println!();
    println!("Version:  {version}");
    println!("  VersionSpec::matches => {spec_match}");
    println!("  Ranges::contains     => {range_match}");
    if spec_match != range_match {
        println!("  ⚠ MISMATCH between VersionSpec and Ranges!");
    }
}

fn print_range_check(
    ranges: &Ranges<Version>,
    against_spec: &VersionSpec,
    against_str: &str,
) -> miette::Result<()> {
    let other_ranges = Ranges::<Version>::try_from(against_spec).into_diagnostic()?;

    println!();
    println!("Against:  {against_str}");
    println!("Parsed:   {against_spec}");
    println!("Ranges:   {}", format_ranges(&other_ranges));
    println!();

    let intersection = ranges.intersection(&other_ranges);
    let is_disjoint = intersection.is_empty();
    let is_subset = ranges.subset_of(&other_ranges);
    let is_superset = other_ranges.subset_of(ranges);
    let are_equal = is_subset && is_superset;

    if are_equal {
        println!("  Equal: the two ranges are identical");
    } else if is_disjoint {
        println!("  Disjoint: no overlap between the two ranges");
    } else if is_subset {
        println!("  Subset: first range is fully contained in second");
    } else if is_superset {
        println!("  Superset: first range fully contains second");
    } else {
        println!("  Overlap: the ranges partially overlap");
    }

    println!("  Intersection: {}", format_ranges(&intersection));
    println!(
        "  Union:        {}",
        format_ranges(&ranges.union(&other_ranges))
    );
    println!();
    println!("  Note: relations are computed on the interval approximation of each spec.");

    println!();
    print_visual_overlap(ranges, &other_ranges);

    Ok(())
}

/// Collect all finite boundary versions from a set of ranges.
fn collect_boundary_points(ranges: &Ranges<Version>, points: &mut BTreeSet<Version>) {
    for (lo, hi) in ranges.iter() {
        if let Bound::Included(v) | Bound::Excluded(v) = lo {
            points.insert(v.clone());
        }
        if let Bound::Included(v) | Bound::Excluded(v) = hi {
            points.insert(v.clone());
        }
    }
}

/// Render a single line of the visual diagram.
fn render_range_line(
    prefix: &str,
    ranges: &Ranges<Version>,
    points: &[Version],
    positions: &[usize],
    left_edge: usize,
    total_width: usize,
) -> String {
    let mut line = vec![' '; total_width];

    for (i, ch) in prefix.chars().enumerate() {
        line[i] = ch;
    }

    for (lo, hi) in ranges.iter() {
        let (start_pos, start_char) = match lo {
            Bound::Unbounded => (left_edge, '<'),
            Bound::Included(v) => {
                let idx = points.iter().position(|p| p == v).unwrap();
                (positions[idx], '[')
            }
            Bound::Excluded(v) => {
                let idx = points.iter().position(|p| p == v).unwrap();
                (positions[idx], '(')
            }
        };

        let (end_pos, end_char) = match hi {
            Bound::Unbounded => (total_width - 1, '>'),
            Bound::Included(v) => {
                let idx = points.iter().position(|p| p == v).unwrap();
                (positions[idx], ']')
            }
            Bound::Excluded(v) => {
                let idx = points.iter().position(|p| p == v).unwrap();
                (positions[idx], ')')
            }
        };

        if start_pos == end_pos {
            line[start_pos] = '●';
        } else {
            line[start_pos] = start_char;
            line[end_pos] = end_char;
            for i in (start_pos + 1)..end_pos {
                if line[i] == ' ' {
                    line[i] = '─';
                }
            }
        }
    }

    line.iter().collect::<String>().trim_end().to_string()
}

fn print_visual_overlap(a: &Ranges<Version>, b: &Ranges<Version>) {
    let intersection = a.intersection(b);

    let mut points = BTreeSet::new();
    collect_boundary_points(a, &mut points);
    collect_boundary_points(b, &mut points);
    collect_boundary_points(&intersection, &mut points);

    let points: Vec<Version> = points.into_iter().collect();
    if points.is_empty() {
        let fmt = |r: &Ranges<Version>| {
            if r.is_empty() {
                "empty"
            } else {
                "(-∞, +∞)"
            }
        };
        println!("  A:   {}", fmt(a));
        println!("  B:   {}", fmt(b));
        println!("  A∩B: {}", fmt(&intersection));
        return;
    }

    let labels: Vec<String> = points.iter().map(|v| v.to_string()).collect();
    // Use the second-shortest label length to set spacing; longer labels get
    // staggered to a row above so they don't crowd the header.
    let mut label_lens: Vec<usize> = labels.iter().map(|l| l.len()).collect();
    label_lens.sort();
    let base_len = if label_lens.len() >= 2 {
        label_lens[1]
    } else {
        label_lens[0]
    };
    let col_spacing = (base_len + 2).max(6);

    const INTERSECTION_PREFIX: &str = "  A∩B: ";
    let prefix_width = INTERSECTION_PREFIX.len();

    // Leave room for unbounded arrows
    let has_unbounded = [a, b, &intersection].iter().any(|r| {
        r.iter()
            .any(|(lo, hi)| matches!(lo, Bound::Unbounded) || matches!(hi, Bound::Unbounded))
    });
    let arrow_pad = if has_unbounded { 3 } else { 0 };

    let left_edge = prefix_width;
    let positions: Vec<usize> = (0..points.len())
        .map(|i| prefix_width + arrow_pad + i * col_spacing)
        .collect();
    // Determine which labels need staggering (would overlap the previous).
    // When two labels collide, stagger the longer one so the compact main
    // row stays readable.
    let mut staggered = vec![false; points.len()];
    let mut main_row_end = 0usize;
    let mut main_row_last = None::<usize>;
    for (i, label) in labels.iter().enumerate() {
        let pos = positions[i];
        if pos < main_row_end + 1 {
            // Collision: stagger whichever label is longer
            if let Some(prev) = main_row_last {
                if labels[prev].len() > label.len() {
                    // Move the previous (longer) label to the stagger row
                    staggered[prev] = true;
                    main_row_end = pos + label.len();
                    main_row_last = Some(i);
                } else {
                    staggered[i] = true;
                }
            } else {
                staggered[i] = true;
            }
        } else {
            main_row_end = pos + label.len();
            main_row_last = Some(i);
        }
    }

    // total_width must fit the longest label (staggered or not)
    let max_label_extent = labels
        .iter()
        .enumerate()
        .map(|(i, l)| positions[i] + l.len())
        .max()
        .unwrap_or(0);
    let total_width = max_label_extent.max(positions.last().unwrap() + arrow_pad + 2);

    // Print stagger row if needed
    if staggered.iter().any(|&s| s) {
        let mut stagger_line = vec![' '; total_width];
        for (i, label) in labels.iter().enumerate() {
            if staggered[i] {
                let pos = positions[i];
                for (j, ch) in label.chars().enumerate() {
                    if pos + j < total_width {
                        stagger_line[pos + j] = ch;
                    }
                }
            }
        }
        println!("{}", stagger_line.iter().collect::<String>().trim_end());
    }

    // Header: version labels (with ↓ for staggered ones)
    let mut header = vec![' '; total_width];
    for (i, label) in labels.iter().enumerate() {
        let pos = positions[i];
        if staggered[i] {
            header[pos] = '↓';
        } else {
            // Don't let the label extend into the next column's ↓ marker
            let max_len = positions
                .get(i + 1)
                .map(|&next| next - pos)
                .unwrap_or(total_width - pos);
            for (j, ch) in label.chars().enumerate() {
                if j >= max_len {
                    break;
                }
                if pos + j < total_width {
                    header[pos + j] = ch;
                }
            }
        }
    }
    println!("{}", header.iter().collect::<String>().trim_end());

    // Range lines
    println!(
        "{}",
        render_range_line("  A:   ", a, &points, &positions, left_edge, total_width)
    );
    println!(
        "{}",
        render_range_line("  B:   ", b, &points, &positions, left_edge, total_width)
    );
    if intersection.is_empty() {
        println!("  A∩B: empty");
    } else {
        println!(
            "{}",
            render_range_line(
                "  A∩B: ",
                &intersection,
                &points,
                &positions,
                left_edge,
                total_width
            )
        );
    }
}

pub fn ranges(opt: Opt) -> miette::Result<()> {
    let spec: VersionSpec = opt.spec.parse().into_diagnostic()?;
    let ranges = Ranges::<Version>::try_from(&spec).into_diagnostic()?;

    println!("Input:    {}", opt.spec);
    println!("Parsed:   {spec}");
    println!("Ranges:   {}", format_ranges(&ranges));

    if let Some(against_str) = &opt.against {
        if let Ok(version) = against_str.parse::<Version>() {
            print_version_check(&spec, &ranges, &version);
        } else {
            let against_spec: VersionSpec = against_str.parse().into_diagnostic()?;
            print_range_check(&ranges, &against_spec, against_str)?;
        }
    }

    Ok(())
}
