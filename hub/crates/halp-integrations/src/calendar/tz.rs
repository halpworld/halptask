//! Time zone names as calendars write them: IANA names (Google, Apple),
//! Windows names (Outlook, Exchange, Microsoft Graph) and vendor-prefixed
//! IANA names (`/mozilla.org/20050126_1/Europe/Stockholm`).

use rrule::Tz;

/// Windows time zone ids → IANA (CLDR `windowsZones.xml`, territory
/// `001`). The common ones; anything else falls back to the calendar's
/// zone.
const WINDOWS: &[(&str, &str)] = &[
    ("Dateline Standard Time", "Etc/GMT+12"),
    ("UTC-11", "Etc/GMT+11"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Pacific Standard Time (Mexico)", "America/Tijuana"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Mountain Standard Time", "America/Denver"),
    ("Central America Standard Time", "America/Guatemala"),
    ("Central Standard Time", "America/Chicago"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Canada Central Standard Time", "America/Regina"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("Eastern Standard Time", "America/New_York"),
    ("US Eastern Standard Time", "America/Indianapolis"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Venezuela Standard Time", "America/Caracas"),
    ("SA Western Standard Time", "America/La_Paz"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("Argentina Standard Time", "America/Buenos_Aires"),
    ("SA Eastern Standard Time", "America/Cayenne"),
    ("Greenland Standard Time", "America/Godthab"),
    ("UTC-02", "Etc/GMT+2"),
    ("Azores Standard Time", "Atlantic/Azores"),
    ("Cape Verde Standard Time", "Atlantic/Cape_Verde"),
    ("UTC", "Etc/UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("W. Central Africa Standard Time", "Africa/Lagos"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Arabic Standard Time", "Asia/Baghdad"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("E. Africa Standard Time", "Africa/Nairobi"),
    ("Iran Standard Time", "Asia/Tehran"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Pakistan Standard Time", "Asia/Karachi"),
    ("India Standard Time", "Asia/Calcutta"),
    ("Sri Lanka Standard Time", "Asia/Colombo"),
    ("Nepal Standard Time", "Asia/Katmandu"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("Cen. Australia Standard Time", "Australia/Adelaide"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("Tasmania Standard Time", "Australia/Hobart"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
];

fn iana(name: &str) -> Option<Tz> {
    name.parse::<chrono_tz::Tz>().ok().map(Tz::Tz)
}

/// The zone a `TZID` names, if this crate knows it.
pub fn resolve(tzid: &str) -> Option<Tz> {
    let t = tzid.trim().trim_matches('"').trim();
    if t.is_empty() || t.len() > 128 {
        return None;
    }
    if matches!(t, "Z" | "UTC" | "GMT" | "Etc/UTC" | "Etc/GMT") {
        return Some(Tz::UTC);
    }
    if let Some(z) = iana(t) {
        return Some(z);
    }
    if let Some((_, z)) = WINDOWS.iter().find(|(w, _)| w.eq_ignore_ascii_case(t)) {
        return iana(z);
    }
    // `/vendor/version/Area/City` or `Area/Sub/City`: try the last three,
    // then the last two segments.
    let parts: Vec<&str> = t.split('/').filter(|p| !p.is_empty()).collect();
    for n in [3usize, 2] {
        if parts.len() >= n
            && let Some(z) = iana(&parts[parts.len() - n..].join("/"))
        {
            return Some(z);
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve() {
        assert_eq!(
            resolve("Europe/Stockholm").unwrap().name(),
            "Europe/Stockholm"
        );
        assert_eq!(
            resolve("W. Europe Standard Time").unwrap().name(),
            "Europe/Berlin"
        );
        assert_eq!(
            resolve("/mozilla.org/20050126_1/America/New_York")
                .unwrap()
                .name(),
            "America/New_York"
        );
        assert_eq!(
            resolve("\"Eastern Standard Time\"").unwrap().name(),
            "America/New_York"
        );
        assert!(resolve("Customized Time Zone").is_none());
        assert!(resolve("").is_none());
    }
}
