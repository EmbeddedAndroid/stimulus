use crate::slot::Sample;

pub fn compress<I>(bits: I) -> Vec<Sample>
where
    I: IntoIterator<Item = u64>,
{
    let mut out: Vec<Sample> = Vec::new();
    for bits in bits {
        if let Some(last) = out.last_mut()
            && last.bits == bits
        {
            last.repeat += 1;
            continue;
        }
        out.push(Sample { bits, repeat: 0 });
    }
    out
}

pub fn expand(samples: &[Sample]) -> impl Iterator<Item = u64> + '_ {
    samples.iter().flat_map(|sample| {
        std::iter::repeat_n(
            sample.bits,
            usize::try_from(sample.repeat.saturating_add(1)).unwrap_or(usize::MAX),
        )
    })
}

pub fn total_len(samples: &[Sample]) -> u64 {
    samples.iter().fold(0u64, |sum, s| {
        sum.saturating_add(s.repeat.saturating_add(1))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn proptest_roundtrip(input in prop::collection::vec(any::<u64>(), 0..4096)) {
            let encoded = compress(input.iter().copied());
            prop_assert_eq!(total_len(&encoded), input.len() as u64);
            prop_assert_eq!(expand(&encoded).collect::<Vec<_>>(), input);
        }
    }
}
