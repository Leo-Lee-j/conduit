//! Domain states retain the legacy string representation on disk. Unknown
//! values round-trip so an older client cannot erase a newer engine state.
macro_rules! status {
    ($name:ident { $($variant:ident => $text:literal),* $(,)? }) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[serde(from="String", into="String")]
        pub enum $name { #[default] Unspecified, $($variant,)* Other(String) }
        impl $name {
            pub fn as_str(&self) -> &str { match self { Self::Unspecified=>"", $(Self::$variant=>$text,)* Self::Other(s)=>s } }
        }
        impl From<String> for $name {
            fn from(s:String)->Self { match s.as_str() { ""=>Self::Unspecified, $($text=>Self::$variant,)* _=>Self::Other(s) } }
        }
        impl From<&str> for $name { fn from(s:&str)->Self { Self::from(s.to_owned()) } }
        impl From<$name> for String { fn from(s:$name)->String { s.as_str().to_owned() } }
        impl std::ops::Deref for $name { type Target=str; fn deref(&self)->&str { self.as_str() } }
        impl std::fmt::Display for $name { fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { f.write_str(self.as_str()) } }
        impl PartialEq<&str> for $name { fn eq(&self,s:&&str)->bool { self.as_str()==*s } }
        impl PartialEq<String> for $name { fn eq(&self,s:&String)->bool { self.as_str()==s } }
    };
}
status!(ToolStatus {
    Running=>"运行中", Output=>"输出中", Completed=>"已完成",
    Cancelled=>"已取消", Interrupted=>"已中断", MissingResult=>"未返回结果", Failed=>"已失败",
});
status!(ApprovalStatus {
    Waiting=>"等待中", Allowed=>"已允许", Denied=>"已拒绝", Answered=>"已回答",
    Skipped=>"已跳过", Cancelled=>"已取消", Interrupted=>"已中断",
});

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_states_and_unknown_future_values_roundtrip() {
        for value in ["运行中", "输出中", "completed", "future-state", ""] {
            let json = serde_json::to_string(value).unwrap();
            let state: ToolStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(serde_json::to_string(&state).unwrap(), json);
        }
        assert_eq!(ApprovalStatus::from("等待中"), ApprovalStatus::Waiting);
    }
}
