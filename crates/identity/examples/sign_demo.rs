//! Imprime chave pública, carga e assinatura para conferir no servidor Node.js (`tools/interop/identity-check.js`).
fn main() {
    let kp = sky_identity::KeyPair::generate();
    let data = sky_identity::payload::auth("abcdefghijk2", "n0nce", "join");
    println!("{}", kp.public_b64().unwrap_or_default());
    println!("{}", kp.sign_b64(&data));
}
