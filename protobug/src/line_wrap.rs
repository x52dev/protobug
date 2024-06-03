use std::{cell::Cell, fmt};

pub(crate) struct LineWrap<T: fmt::Display> {
    pub(crate) string: T,
    pub(crate) wrap_at: usize,

    /// Tracks lines in output during formatting.
    pub(crate) lines: Cell<usize>,
}

impl<T: fmt::Display> LineWrap<T> {
    pub(crate) fn new(string: T, wrap_at: usize) -> Self {
        Self {
            string,
            wrap_at,
            lines: Cell::new(0),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn wrapped(self) -> (String, usize) {
        (self.to_string(), self.lines.get())
    }
}

impl<T: fmt::Display> fmt::Display for LineWrap<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = self.string.to_string();

        loop {
            self.lines.set(self.lines.get() + 1);

            let (line, rest) = split_to(&mut buf, self.wrap_at);

            writeln!(f, "{}", &line)?;

            match rest {
                Some(rest) => buf = rest,
                None => break,
            }
        }

        Ok(())
    }
}

fn split_to(string: &mut String, pos: usize) -> (&mut String, Option<String>) {
    let pos = if pos >= string.len() {
        return (string, None);
    } else {
        pos
    };

    let rest = string.split_off(pos);

    (string, Some(rest))
}

#[cfg(test)]
pub(crate) mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn string_split_to() {
        assert_eq!(split_to(&mut "".to_owned(), 0), (&mut "".to_owned(), None));
        assert_eq!(split_to(&mut "".to_owned(), 1), (&mut "".to_owned(), None));

        assert_eq!(
            split_to(&mut "foobar".to_owned(), 0),
            (&mut "".to_owned(), Some("foobar".to_owned())),
        );

        assert_eq!(
            split_to(&mut "foobar".to_owned(), 10),
            (&mut "foobar".to_owned(), None),
        );

        assert_eq!(
            split_to(&mut "foobar".to_owned(), 3),
            (&mut "foo".to_owned(), Some("bar".to_owned())),
        );

        // panics
        // assert_eq!(
        //     split_to(&mut "née".to_owned(), 2),
        //     (&mut "né".to_owned(), Some("e".to_owned())),
        // );
    }

    #[test]
    fn line_wrapper() {
        assert_eq!(LineWrap::new("", 0).to_string(), "\n");
        assert_eq!(LineWrap::new("", 1).to_string(), "\n");

        assert_eq!(LineWrap::new("foo", 3).to_string(), "foo\n");
        assert_eq!(LineWrap::new("foo", 6).to_string(), "foo\n");

        assert_eq!(LineWrap::new("foobar", 3).to_string(), "foo\nbar\n");
    }
}
