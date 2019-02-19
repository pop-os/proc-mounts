use partition_identity::PartitionID;
use std::char;
use std::ffi::OsString;
use std::fmt::{self, Display, Formatter};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Error, ErrorKind};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// A mount entry which contains information regarding how and where a source
/// is mounted.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct MountInfo {
    /// The source which is mounted.
    pub source: PathBuf,
    /// Where the source is mounted.
    pub dest: PathBuf,
    /// The type of the mounted file system.
    pub fstype: String,
    /// Options specified for this file system.
    pub options: Vec<String>,
    /// Defines if the file system should be dumped.
    pub dump: i32,
    /// Defines if the file system should be checked, and in what order.
    pub pass: i32,
}

impl Display for MountInfo {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        write!(
            fmt,
            "{} {} {} {} {} {}",
            self.source.display(),
            self.dest.display(),
            self.fstype,
            self.options.join(","),
            self.dump,
            self.pass
        )
    }
}

impl FromStr for MountInfo {
    type Err = io::Error;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let mut parts = line.split_whitespace();

        fn map_err(why: &'static str) -> io::Error {
            Error::new(ErrorKind::InvalidData, why)
        }

        let source = parts.next().ok_or_else(|| map_err("missing source"))?;
        let dest = parts.next().ok_or_else(|| map_err("missing dest"))?;
        let fstype = parts.next().ok_or_else(|| map_err("missing type"))?;
        let options = parts.next().ok_or_else(|| map_err("missing options"))?;

        let dump = parts
            .next()
            .ok_or_else(|| map_err("missing dump"))?
            .parse::<i32>()
            .map_err(|_| map_err("dump value is not a number"))?;
        let pass = parts
            .next()
            .ok_or_else(|| map_err("missing pass"))?
            .trim_right()
            .parse::<i32>()
            .map_err(|_| map_err("pass value is not a number"))?;

        let path = Self::parse_value(source)?;
        let path = path.to_str().ok_or_else(|| map_err("non-utf8 paths are unsupported"))?;

        let source = if path.starts_with("/dev/disk/by-") {
            Self::fetch_from_disk_by_path(path)?
        } else {
            PathBuf::from(path)
        };

        let path = Self::parse_value(dest)?;
        let path = path.to_str().ok_or_else(|| map_err("non-utf8 paths are unsupported"))?;

        let dest = PathBuf::from(path);

        Ok(MountInfo {
            source,
            dest,
            fstype: fstype.to_owned(),
            options: options.split(',').map(String::from).collect(),
            dump,
            pass,
        })
    }
}

impl MountInfo {
    /// Attempt to parse a `/proc/mounts`-like line.
    #[deprecated]
    pub fn parse_line(line: &str) -> io::Result<MountInfo> {
        line.parse::<Self>()
    }

    fn fetch_from_disk_by_path(path: &str) -> io::Result<PathBuf> {
        PartitionID::from_disk_by_path(path)
            .map_err(|why| Error::new(ErrorKind::InvalidData, format!("{}: {}", path, why)))?
            .get_device_path()
            .ok_or_else(|| {
                Error::new(ErrorKind::NotFound, format!("device path for {} was not found", path))
            })
    }

    fn parse_value(value: &str) -> io::Result<OsString> {
        let mut ret = Vec::new();

        let mut bytes = value.bytes();
        while let Some(b) = bytes.next() {
            match b {
                b'\\' => {
                    let mut code = 0;
                    for _i in 0..3 {
                        if let Some(b) = bytes.next() {
                            code *= 8;
                            code += u32::from_str_radix(&(b as char).to_string(), 8)
                                .map_err(|err| Error::new(ErrorKind::Other, err))?;
                        } else {
                            return Err(Error::new(ErrorKind::Other, "truncated octal code"));
                        }
                    }
                    ret.push(code as u8);
                }
                _ => {
                    ret.push(b);
                }
            }
        }

        Ok(OsString::from_vec(ret))
    }
}

/// A list of parsed mount entries from `/proc/mounts`.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct MountList(pub Vec<MountInfo>);

impl MountList {
    /// Parse mounts given from an iterator of mount entry lines.
    pub fn parse_from<'a, I: Iterator<Item = &'a str>>(lines: I) -> io::Result<MountList> {
        lines.map(MountInfo::from_str).collect::<io::Result<Vec<MountInfo>>>().map(MountList)
    }

    /// Read a new list of mounts into memory from `/proc/mounts`.
    pub fn new() -> io::Result<MountList> {
        Ok(MountList(MountIter::new()?.collect::<io::Result<Vec<MountInfo>>>()?))
    }

    /// Read a new list of mounts into memory from any mount-tab-like file.
    pub fn new_from_file<P: AsRef<Path>>(path: P) -> io::Result<MountList> {
        Ok(MountList(MountIter::new_from_file(path)?.collect::<io::Result<Vec<MountInfo>>>()?))
    }

    /// Read a new list of mounts into memory from any mount-tab-like file.
    pub fn new_from_reader<R: BufRead>(reader: R) -> io::Result<MountList> {
        Ok(MountList(MountIter::new_from_reader(reader).collect::<io::Result<Vec<MountInfo>>>()?))
    }

    // Returns true if the `source` is mounted at the given `dest`.
    pub fn source_mounted_at<D: AsRef<Path>, P: AsRef<Path>>(&self, source: D, path: P) -> bool {
        self.get_mount_by_source(source)
            .map_or(false, |mount| mount.dest.as_path() == path.as_ref())
    }

    /// Find the first mount which which has the `path` destination.
    pub fn get_mount_by_dest<P: AsRef<Path>>(&self, path: P) -> Option<&MountInfo> {
        self.0.iter().find(|mount| mount.dest == path.as_ref())
    }

    /// Find the first mount hich has the source `path`.
    pub fn get_mount_by_source<P: AsRef<Path>>(&self, path: P) -> Option<&MountInfo> {
        self.0.iter().find(|mount| mount.source == path.as_ref())
    }

    /// Iterate through each source that starts with the given `path`.
    pub fn source_starts_with<'a>(
        &'a self,
        path: &'a Path,
    ) -> Box<Iterator<Item = &MountInfo> + 'a> {
        self.starts_with(path.as_os_str().as_bytes(), |m| &m.source)
    }

    /// Iterate through each destination that starts with the given `path`.
    pub fn destination_starts_with<'a>(
        &'a self,
        path: &'a Path,
    ) -> Box<Iterator<Item = &MountInfo> + 'a> {
        self.starts_with(path.as_os_str().as_bytes(), |m| &m.dest)
    }

    fn starts_with<'a, F: Fn(&'a MountInfo) -> &'a Path + 'a>(
        &'a self,
        path: &'a [u8],
        func: F,
    ) -> Box<Iterator<Item = &MountInfo> + 'a> {
        let iterator = self.0.iter().filter(move |mount| {
            let input = func(mount).as_os_str().as_bytes();
            input.len() >= path.len() && &input[..path.len()] == path
        });

        Box::new(iterator)
    }
}

/// Iteratively parse the `/proc/mounts` file.
pub struct MountIter<R> {
    file: R,
    buffer: String,
}

impl MountIter<BufReader<File>> {
    pub fn new() -> io::Result<Self> {
        Self::new_from_file("/proc/mounts")
    }

    /// Read mounts from any mount-tab-like file.
    pub fn new_from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Ok(Self::new_from_reader(BufReader::new(File::open(path)?)))
    }
}

impl<R: BufRead> MountIter<R> {
    /// Read mounts from any in-memory buffer.
    pub fn new_from_reader(readable: R) -> Self {
        Self { file: readable, buffer: String::with_capacity(512) }
    }

    /// Iterator-based variant of `source_mounted_at`.
    ///
    /// Returns true if the `source` is mounted at the given `dest`.
    ///
    /// Due to iterative parsing of the mount file, an error may be returned.
    pub fn source_mounted_at<D: AsRef<Path>, P: AsRef<Path>>(
        source: D,
        path: P,
    ) -> io::Result<bool> {
        let source = source.as_ref();
        let path = path.as_ref();

        let mut is_found = false;

        let mounts = MountIter::new()?;
        for mount in mounts {
            let mount = mount?;
            if mount.source == source {
                is_found = mount.dest == path;
                break;
            }
        }

        Ok(is_found)
    }
}

impl<R: BufRead> Iterator for MountIter<R> {
    type Item = io::Result<MountInfo>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.buffer.clear();
            match self.file.read_line(&mut self.buffer) {
                Ok(read) if read == 0 => return None,
                Ok(_) => {
                    let line = self.buffer.trim_left();
                    if !(line.starts_with('#') || line.is_empty()) {
                        return Some(MountInfo::from_str(line));
                    }
                }
                Err(why) => return Some(Err(why)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const SAMPLE: &str = r#"sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0
proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0
udev /dev devtmpfs rw,nosuid,relatime,size=16420480k,nr_inodes=4105120,mode=755 0 0
tmpfs /run tmpfs rw,nosuid,noexec,relatime,size=3291052k,mode=755 0 0
/dev/sda2 / ext4 rw,noatime,errors=remount-ro,data=ordered 0 0
fusectl /sys/fs/fuse/connections fusectl rw,relatime 0 0
/dev/sda1 /boot/efi vfat rw,relatime,fmask=0077,dmask=0077,codepage=437,iocharset=iso8859-1,shortname=mixed,errors=remount-ro 0 0
/dev/sda6 /mnt/data ext4 rw,noatime,data=ordered 0 0"#;

    #[test]
    fn source_mounted_at() {
        let mounts = MountList::parse_from(SAMPLE.lines()).unwrap();
        assert!(mounts.source_mounted_at("/dev/sda2", "/"));
        assert!(mounts.source_mounted_at("/dev/sda1", "/boot/efi"));
    }

    #[test]
    fn mounts() {
        let mounts = MountList::parse_from(SAMPLE.lines()).unwrap();

        assert_eq!(
            mounts.get_mount_by_source(Path::new("/dev/sda1")).unwrap(),
            &MountInfo {
                source: PathBuf::from("/dev/sda1"),
                dest: PathBuf::from("/boot/efi"),
                fstype: "vfat".into(),
                options: vec![
                    "rw".into(),
                    "relatime".into(),
                    "fmask=0077".into(),
                    "dmask=0077".into(),
                    "codepage=437".into(),
                    "iocharset=iso8859-1".into(),
                    "shortname=mixed".into(),
                    "errors=remount-ro".into(),
                ],
                dump: 0,
                pass: 0,
            }
        );

        let path = &Path::new("/");
        assert_eq!(
            mounts.destination_starts_with(path).map(|m| m.dest.clone()).collect::<Vec<_>>(),
            {
                let mut vec: Vec<PathBuf> = Vec::new();
                vec.push("/sys".into());
                vec.push("/proc".into());
                vec.push("/dev".into());
                vec.push("/run".into());
                vec.push("/".into());
                vec.push("/sys/fs/fuse/connections".into());
                vec.push("/boot/efi".into());
                vec.push("/mnt/data".into());
                vec
            }
        );
    }
}
