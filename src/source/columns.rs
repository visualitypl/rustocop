//! Lazy scalar columns: sparse blocks of byte-to-character savings.
//!
//! Build once in O(bytes + lines) work. Each physical line is divided at Ruby
//! character boundaries into blocks of at most 256 bytes (Prism's widest source
//! character is six bytes). Store only blocks containing multibyte characters:
//! ASCII and declared single-byte files allocate no block buffer. Storage is
//! O(bytes / 256 + lines), not one entry per byte/character/query. Queries cost
//! two binary searches plus at most 256 bytes of prefix counting, independent
//! of query order. Parser diagnostics retain their separate batched mapper.

use super::encoding::ParsedEncoding;

const BLOCK_BYTES: usize = 256;

#[derive(Default)]
pub(super) struct ColumnIndex {
    blocks: Vec<Block>,
}

struct Block {
    start: usize,
    end: usize,
    /// Cumulative savings before this block, across physical lines.
    before: usize,
    after: usize,
}

impl ColumnIndex {
    pub fn build(bytes: &[u8], line_starts: &[usize], encoding: ParsedEncoding) -> Self {
        let mut index = Self::default();
        if encoding.single_byte() {
            return index;
        }
        let mut savings = 0;
        for (line, &raw_start) in line_starts.iter().enumerate() {
            let mut cursor = if line == 0 && bytes.starts_with(b"\xef\xbb\xbf") {
                3
            } else {
                raw_start
            };
            let end = line_starts.get(line + 1).copied().unwrap_or(bytes.len());
            while cursor < end {
                let start = cursor;
                let before = savings;
                while cursor < end {
                    let width = encoding.width(&bytes[cursor..end]);
                    if cursor - start + width > BLOCK_BYTES && cursor > start {
                        break;
                    }
                    cursor += width;
                    savings += width - 1;
                }
                if savings != before {
                    index.blocks.push(Block {
                        start,
                        end: cursor,
                        before,
                        after: savings,
                    });
                }
            }
        }
        index
    }

    /// Count a physical-line prefix, whose start excludes a possible BOM.
    pub fn count_chars(
        &self,
        bytes: &[u8],
        start: usize,
        end: usize,
        encoding: ParsedEncoding,
    ) -> usize {
        end - start
            - (self.savings_at(bytes, end, encoding) - self.savings_at(bytes, start, encoding))
    }

    fn savings_at(&self, bytes: &[u8], offset: usize, encoding: ParsedEncoding) -> usize {
        let preceding = self.blocks.partition_point(|block| block.start < offset);
        let Some(block) = preceding.checked_sub(1).map(|i| &self.blocks[i]) else {
            return 0;
        };
        if offset >= block.end {
            block.after
        } else {
            // Truncating here intentionally re-counts incomplete characters as
            // invalid bytes, exactly like the old full-line prefix mapper.
            block.before + offset - block.start - encoding.count_chars(&bytes[block.start..offset])
        }
    }

    #[cfg(test)]
    pub fn allocated_blocks(&self) -> (usize, usize) {
        (self.blocks.len(), self.blocks.capacity())
    }
}
