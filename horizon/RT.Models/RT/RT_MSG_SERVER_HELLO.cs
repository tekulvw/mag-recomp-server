using Org.BouncyCastle.Math;
using RT.Common;
using Server.Common;
using System;
using System.Collections.Generic;
using System.IO;
using System.Text;

namespace RT.Models
{
    [ScertMessage(RT_MSG_TYPE.RT_MSG_SERVER_HELLO)]
    public class RT_MSG_SERVER_HELLO : BaseScertMessage
    {
        public override RT_MSG_TYPE Id => RT_MSG_TYPE.RT_MSG_SERVER_HELLO;

        public ushort ARG1 = 0x006E;
        public ushort ARG2 = 0x0001;

        //PS3 
        public bool MLS = false;
        public byte EncryptFlagDisable = 0x02;

        public byte[] Certificate;
        public byte[] MLSCert;
        public BigInteger RsaPublicKey = BigInteger.Zero;

        public RT_MSG_SERVER_HELLO()
        {

        }

        // Certificate templates are runtime inputs; no captured or vendor certificate
        // bytes are distributed with this source. Clone caller inputs before patching.
        private static byte[] LoadCertificate(byte[] supplied, string variable, int minimumLength)
        {
            var bytes = supplied;
            if (bytes == null)
            {
                var path = Environment.GetEnvironmentVariable(variable);
                if (string.IsNullOrWhiteSpace(path))
                    throw new InvalidOperationException("PS3 SCERT hello requires an external certificate template: " + variable);
                bytes = File.ReadAllBytes(path);
            }
            if (bytes.Length < minimumLength)
                throw new InvalidDataException("SCERT certificate template is too short: " + variable);
            return (byte[])bytes.Clone();
        }

        public override void Deserialize(Server.Common.Stream.MessageReader reader)
        {
            ARG1 = reader.ReadUInt16();
            ARG2 = reader.ReadUInt16();
        }

        public override void Serialize(Server.Common.Stream.MessageWriter writer)
        {
            //If PS3 Medius Version 112/113, Send PS3 MAS Cert for Encryption/Decryption
            if ((writer.MediusVersion == 112 || writer.MediusVersion == 113) && MLS == false)
            {
                // serialize rsa modulus
                // this is sent in server hello at offset 0x194
                // we're going to overwrite the cert at that offset to store the rsa modulus
                var rsakey = RsaPublicKey.ToByteArrayUnsigned();

                // fix to 64 bytes (512 bit)
                Array.Resize(ref rsakey, 0x40);

                // copy to cert
                var certificate = LoadCertificate(Certificate, "HORIZON_SCERT_MAS_CERTIFICATE_PATH", 0x1d4);
                Array.Copy(rsakey, 0, certificate, 0x194, rsakey.Length);

                // write
                writer.Write(certificate);
            }
            //If PS3 Medius Version 112/113, and is MLS, Send PS3 MLS Cert for Encryption/Decryption
            else if ((writer.MediusVersion == 112 || writer.MediusVersion == 113) && MLS == true)
            {
                writer.Write(EncryptFlagDisable);
                writer.Write(LoadCertificate(MLSCert, "HORIZON_SCERT_MLS_CERTIFICATE_PATH", 1));
            }
            else //Send PS2 Server Hello
            {
                writer.Write(ARG1);
                writer.Write(ARG2);
            }
        }
    }
}
