//! "Did you mean?" suggestions for misspelt names: a port of Ruby's
//! `DidYouMean::SpellChecker` (Jaro-Winkler similarity, then Levenshtein
//! distance), which RuboCop's `NameSimilarity` uses. did_you_mean is part of
//! Ruby (BSD-2-Clause/Ruby license); its Levenshtein code derives from the
//! Text gem (MIT).

/// The most similar of `names` to `target`, as RuboCop's
/// `NameSimilarity.find_similar_name` picks it; `target` itself never is.
pub fn find_similar_name<'a>(target: &str, names: &[&'a str]) -> Option<&'a str> {
    let names: Vec<&'a str> = names
        .iter()
        .copied()
        .filter(|&name| name != target)
        .collect();
    correct(target, &names).into_iter().next()
}

/// `DidYouMean::SpellChecker#correct`.
fn correct<'a>(input: &str, dictionary: &[&'a str]) -> Vec<&'a str> {
    let normalized_input = normalize(input);
    let input_length = normalized_input.len();
    let threshold = if input_length > 3 { 0.834 } else { 0.77 };
    let mut words: Vec<&'a str> = dictionary
        .iter()
        .copied()
        .filter(|word| jaro_winkler(&normalize(word), &normalized_input) >= threshold)
        .filter(|&word| word != input)
        .collect();
    let word_chars = |word: &str| word.chars().collect::<Vec<char>>();
    // Most similar first (a stable sort, then reversed, as Ruby does).
    words.sort_by(|a, b| {
        let a = jaro_winkler(&word_chars(a), &normalized_input);
        let b = jaro_winkler(&word_chars(b), &normalized_input);
        a.total_cmp(&b)
    });
    words.reverse();

    // Mistypes.
    let threshold = (input_length as f64 * 0.25).ceil() as usize;
    let corrections: Vec<&'a str> = words
        .iter()
        .copied()
        .filter(|word| levenshtein(&normalize(word), &normalized_input) <= threshold)
        .collect();
    if !corrections.is_empty() {
        return corrections;
    }
    // Misspellings.
    words
        .into_iter()
        .filter(|word| {
            let word = normalize(word);
            let length = input_length.min(word.len());
            levenshtein(&word, &normalized_input) < length
        })
        .take(1)
        .collect()
}

/// Lowercased, without `@`.
fn normalize(name: &str) -> Vec<char> {
    name.to_lowercase().chars().filter(|&c| c != '@').collect()
}

/// `DidYouMean::Jaro.distance`.
fn jaro(a: &[char], b: &[char]) -> f64 {
    let (a, b) = if a.len() > b.len() { (b, a) } else { (a, b) };
    let (length1, length2) = (a.len(), b.len());
    let range = (length2 / 2).saturating_sub(1);
    let mut flags1 = vec![false; length1];
    let mut flags2 = vec![false; length2];
    let mut matches = 0.0;
    for i in 0..length1 {
        let last = i + range;
        let mut j = i.saturating_sub(range);
        while j <= last && j < length2 {
            if !flags2[j] && a[i] == b[j] {
                flags2[j] = true;
                flags1[i] = true;
                matches += 1.0;
                break;
            }
            j += 1;
        }
    }
    let mut transpositions = 0.0;
    let mut k = 0;
    for i in 0..length1 {
        if !flags1[i] {
            continue;
        }
        let mut index = k;
        let mut j = k;
        while j < length2 {
            index = j;
            if flags2[j] {
                k = j + 1;
                break;
            }
            j += 1;
        }
        if b.get(index) != Some(&a[i]) {
            transpositions += 1.0;
        }
    }
    let transpositions = (transpositions / 2.0f64).floor();
    if matches == 0.0 {
        return 0.0;
    }
    (matches / length1 as f64 + matches / length2 as f64 + (matches - transpositions) / matches)
        / 3.0
}

/// `DidYouMean::JaroWinkler.distance`.
fn jaro_winkler(a: &[char], b: &[char]) -> f64 {
    let distance = jaro(a, b);
    if distance <= 0.7 {
        return distance;
    }
    let prefix = a
        .iter()
        .zip(b.iter())
        .take(4)
        .take_while(|(x, y)| x == y)
        .count();
    distance + prefix as f64 * 0.1 * (1.0 - distance)
}

/// The Levenshtein edit distance.
pub(crate) fn levenshtein(a: &[char], b: &[char]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &x) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, &y) in b.iter().enumerate() {
            let substitution = previous + usize::from(x != y);
            previous = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_mistyped_names() {
        assert_eq!(
            find_similar_name("enviromnent", &["another_symbol", "environment", "puts"]),
            Some("environment")
        );
        assert_eq!(find_similar_name("item", &["items", "item"]), Some("items"));
        assert_eq!(
            find_similar_name("enviromnent", &["another_symbol", "envelope", "puts"]),
            None
        );
        assert_eq!(find_similar_name("j", &["i", "items"]), None);
        assert_eq!(find_similar_name("foo", &[]), None);
    }

    #[test]
    fn distances_match_did_you_mean() {
        let chars = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(levenshtein(&chars("kitten"), &chars("sitting")), 3);
        assert_eq!(levenshtein(&chars(""), &chars("abc")), 3);
        // DidYouMean::JaroWinkler.distance("martha", "marhta") == 0.9611111111111111
        let distance = jaro_winkler(&chars("martha"), &chars("marhta"));
        assert!(
            (distance - 0.961_111_111_111_111_1).abs() < 1e-12,
            "{distance}"
        );
        // DidYouMean::Jaro.distance("dwayne", "duane") == 0.8222222222222223
        let distance = jaro(&chars("dwayne"), &chars("duane"));
        assert!(
            (distance - 0.822_222_222_222_222_3).abs() < 1e-12,
            "{distance}"
        );
    }
}
