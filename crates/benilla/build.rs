//! Stamps the commit this binary was built from, and gives it World of Kartcraft's icon.

fn main() {
    benilla_buildstamp::emit();
    benilla_buildstamp::windows_icon();
}
