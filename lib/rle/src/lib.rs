pub fn decode(data: &[u8], output: &mut Vec<u8>) {
    let mut i = 1;
    let mut cur = data[0];
    let mut len = 0;
    while i <= data.len() {
        while data.get(i) == Some(&cur) && len < 127 {
            len += 1;
            i += 1;
        }

        if len != 0 {
            output.push(len);
            output.push(cur);
            cur = data.get(i).cloned().unwrap_or(0);
            i += 1;
            len = 0;
        }

        while match data.get(i) {
            Some(&ch) if ch != cur && len < 127 => {
                len += 1;
                cur = ch;
                true
            }
            _ => false,
        } {}

        if len != 0 {
            output.push(0x80 + (len - 1));
            output.extend_from_slice(&data[i - (len - 1) as usize..i - 1]);
            output.push(cur);
            len = 0;
        }
    }
}
