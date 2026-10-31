using System;
using Org.BouncyCastle.Crypto.Generators;
using Org.BouncyCastle.Crypto.Parameters;
using Org.BouncyCastle.Math;
using Org.BouncyCastle.Security;

namespace RT.Cryptography
{
    /// <summary>
    /// Ephemeral defaults for isolated development. The legacy wire format requires
    /// a 512-bit modulus and exponent 17; these are not modern production TLS keys.
    /// Operators must provide compatible keys externally for persistent deployments.
    /// </summary>
    public static class DevelopmentRsaKeys
    {
        private static readonly Lazy<RsaKeyPair> ProcessKey = new Lazy<RsaKeyPair>(Generate);
        public static RsaKeyPair ForProcess => ProcessKey.Value;

        private static RsaKeyPair Generate()
        {
            var generator = new RsaKeyPairGenerator();
            var exponent = BigInteger.ValueOf(17);
            generator.Init(new RsaKeyGenerationParameters(exponent, new SecureRandom(), 512, 80));
            var pair = generator.GenerateKeyPair();
            var publicKey = (RsaKeyParameters)pair.Public;
            var privateKey = (RsaKeyParameters)pair.Private;
            return new RsaKeyPair(publicKey.Modulus, exponent, privateKey.Exponent);
        }
    }
}
