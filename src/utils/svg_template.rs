use crate::error::{Error, Result};
use minijinja::{Environment, UndefinedBehavior, Value};
use serde::Serialize;
use std::{fs, path::Path};

/// A MiniJinja template that is parsed **once**, at load time.
///
/// The previous version built a fresh `Environment` and re-parsed the whole SVG
/// on every render call. That was only ~36 µs per frame, but it also meant a
/// syntax error in the template surfaced once per frame instead of once at
/// startup, which is the more useful half of this change.
pub struct SvgTemplate {
    env: Environment<'static>,
}

impl SvgTemplate {
    /// Рендерит SVG с подстановкой переменных через MiniJinja.
    ///
    /// # Поддерживаемые конструкции в SVG:
    /// - `{{ variable }}` — подстановка значения
    /// - `{% if cond %}...{% else %}...{% endif %}` — условие
    /// - `{% for i in range(n) %}...{% endfor %}` — цикл
    /// - `{{ a + b }}`, `{{ a * b }}`, `{{ a // b }}` — арифметика
    ///
    /// # Пример SVG-шаблона:
    /// ```xml
    /// <text>{{ cpu_temp }}°C</text>
    /// <rect width="{{ cpu_usage * bar_track_width // 100 }}" height="8" fill="white"/>
    /// {% for i in range(10) %}
    ///   <rect x="{{ i * 24 }}" fill="{% if i < ram_usage // 10 %}white{% else %}none{% endif %}"/>
    /// {% endfor %}
    /// ```
    ///
    /// Undefined variables are an error, not an empty string: a typo used to
    /// render as a blank attribute and silently break the layout.
    pub fn load<P: AsRef<Path>>(source: P) -> Result<Self> {
        let path = source.as_ref();
        let text = fs::read_to_string(path)
            .map_err(|e| Error::Asset(format!("failed to read SVG template {}: {e}", path.display())))?;

        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        env.add_template_owned("svg", text)
            .map_err(|e| Error::Asset(format!("SVG template error in {}: {e}", path.display())))?;

        Ok(Self { env })
    }

    pub fn render(&self, ctx: &impl Serialize) -> Result<String> {
        let tmpl = self
            .env
            .get_template("svg")
            .map_err(|e| Error::Asset(format!("SVG template error: {e}")))?;
        tmpl.render(Value::from_serialize(ctx))
            .map_err(|e| Error::Asset(format!("SVG render error: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn template_from(body: &str) -> Result<SvgTemplate> {
        let mut f = tempfile();
        f.write_all(body.as_bytes()).unwrap();
        SvgTemplate::load(f.path())
    }

    struct TempSvg {
        path: std::path::PathBuf,
        file: Option<std::fs::File>,
    }
    impl TempSvg {
        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }
    impl Write for TempSvg {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.file.as_mut().unwrap().write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.as_mut().unwrap().flush()
        }
    }
    impl Drop for TempSvg {
        fn drop(&mut self) {
            self.file.take();
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn tempfile() -> TempSvg {
        let mut path = std::env::temp_dir();
        // Unique enough for a single-process test run.
        path.push(format!(
            "deepcool-tmpl-{}-{:?}.svg",
            std::process::id(),
            std::thread::current().id()
        ));
        let file = std::fs::File::create(&path).unwrap();
        TempSvg { path, file: Some(file) }
    }

    #[derive(serde::Serialize)]
    struct Ctx {
        cpu_temp: u8,
        bar_track_width: u32,
    }

    #[test]
    fn substitutes_values_and_arithmetic() {
        let t = template_from("<text>{{ cpu_temp }}</text><rect width=\"{{ cpu_temp * bar_track_width // 100 }}\"/>")
            .unwrap();
        let out = t
            .render(&Ctx {
                cpu_temp: 50,
                bar_track_width: 296,
            })
            .unwrap();
        assert!(out.contains("<text>50</text>"), "{out}");
        assert!(out.contains("width=\"148\""), "{out}");
    }

    #[test]
    fn an_unknown_variable_is_an_error_not_an_empty_attribute() {
        let t = template_from("<text>{{ definitely_not_a_metric }}</text>").unwrap();
        let err = t
            .render(&Ctx {
                cpu_temp: 1,
                bar_track_width: 1,
            })
            .unwrap_err();
        assert!(matches!(err, Error::Asset(_)));
    }

    #[test]
    fn a_syntax_error_is_reported_at_load_time() {
        assert!(template_from("<text>{{ unclosed </text>").is_err());
    }
}
